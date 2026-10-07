//! The queen — a tiny word-level GPT, pretrained from scratch (random init) on
//! nothing but the crawl corpus, in pure Rust on the CPU via candle.
//!
//! Word-level (not character-level) so that even a lightly-trained model emits
//! real security vocabulary rather than letter soup. She is deliberately small
//! so she can be pretrained on a plain VPS and retrained as the corpus grows —
//! crude at first, better the more the hatchlings read.

use anyhow::{anyhow, Result};
use candle_core::{DType, Device, IndexOp, Tensor, D};
use candle_nn::{
    embedding, layer_norm, linear, loss::cross_entropy, ops::softmax, AdamW, Embedding, LayerNorm,
    Linear, Module, Optimizer, ParamsAdamW, VarBuilder, VarMap,
};
use rand::distributions::{Distribution, WeightedIndex};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

const MAX_VOCAB: usize = 4096;
const TOP_K: usize = 40;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Config {
    pub vocab_size: usize,
    pub block_size: usize,
    pub n_embd: usize,
    pub n_head: usize,
    pub n_layer: usize,
}

impl Config {
    pub fn small(vocab_size: usize) -> Self {
        Config { vocab_size, block_size: 96, n_embd: 160, n_head: 5, n_layer: 4 }
    }
    pub fn n_params(&self) -> u64 {
        let d = self.n_embd as u64;
        let v = self.vocab_size as u64;
        let per_layer = 12 * d * d;
        v * d + (self.block_size as u64) * d + self.n_layer as u64 * per_layer + d * v
    }
}

// --- word vocabulary --------------------------------------------------------

/// Split text into word / punctuation tokens. Whitespace separates; runs of
/// alphanumerics (plus `-` and `_`) are one token; every other non-space char
/// is its own token.
pub fn tokenize(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in text.chars() {
        if c.is_alphanumeric() || c == '-' || c == '_' {
            cur.push(c);
        } else {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            if !c.is_whitespace() {
                out.push(c.to_string());
            }
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Vocab {
    pub words: Vec<String>, // index 0 is always "<unk>"
}

impl Vocab {
    pub fn from_text(text: &str, max_vocab: usize) -> Self {
        let mut freq: HashMap<String, u32> = HashMap::new();
        for w in tokenize(text) {
            *freq.entry(w).or_insert(0) += 1;
        }
        let mut items: Vec<(String, u32)> = freq.into_iter().collect();
        items.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let mut words = vec!["<unk>".to_string()];
        for (w, _) in items.into_iter().take(max_vocab.saturating_sub(1)) {
            words.push(w);
        }
        Vocab { words }
    }
    pub fn len(&self) -> usize {
        self.words.len()
    }
    pub fn is_empty(&self) -> bool {
        self.words.len() <= 1
    }
    fn index(&self) -> HashMap<&str, u32> {
        self.words
            .iter()
            .enumerate()
            .map(|(i, w)| (w.as_str(), i as u32))
            .collect()
    }
    pub fn encode(&self, text: &str) -> Vec<u32> {
        let idx = self.index();
        tokenize(text)
            .iter()
            .map(|w| idx.get(w.as_str()).copied().unwrap_or(0))
            .collect()
    }
    /// Join tokens back into readable text (no space before closing punctuation).
    pub fn decode(&self, ids: &[u32]) -> String {
        let mut out = String::new();
        for &id in ids {
            let Some(w) = self.words.get(id as usize) else { continue };
            if w == "<unk>" {
                continue;
            }
            let no_space_before = w.len() == 1
                && matches!(
                    w.chars().next().unwrap(),
                    ',' | '.' | ';' | ':' | ')' | ']' | '}' | '!' | '?' | '\'' | '"' | '%'
                );
            if out.is_empty() || no_space_before {
                out.push_str(w);
            } else {
                out.push(' ');
                out.push_str(w);
            }
        }
        out
    }
}

// --- model ------------------------------------------------------------------

struct Mlp {
    fc: Linear,
    proj: Linear,
}
impl Mlp {
    fn new(d: usize, vb: VarBuilder) -> Result<Self> {
        Ok(Self {
            fc: linear(d, 4 * d, vb.pp("fc"))?,
            proj: linear(4 * d, d, vb.pp("proj"))?,
        })
    }
    fn forward(&self, x: &Tensor) -> Result<Tensor> {
        Ok(self.proj.forward(&self.fc.forward(x)?.gelu()?)?)
    }
}

struct Attn {
    qkv: Linear,
    proj: Linear,
    n_head: usize,
}
impl Attn {
    fn new(cfg: &Config, vb: VarBuilder) -> Result<Self> {
        Ok(Self {
            qkv: linear(cfg.n_embd, 3 * cfg.n_embd, vb.pp("qkv"))?,
            proj: linear(cfg.n_embd, cfg.n_embd, vb.pp("proj"))?,
            n_head: cfg.n_head,
        })
    }
    fn forward(&self, x: &Tensor, mask: &Tensor) -> Result<Tensor> {
        let (b, t, c) = x.dims3()?;
        let hd = c / self.n_head;
        let qkv = self.qkv.forward(x)?;
        let q = qkv.narrow(2, 0, c)?;
        let k = qkv.narrow(2, c, c)?;
        let v = qkv.narrow(2, 2 * c, c)?;
        let split = |t_: &Tensor| -> Result<Tensor> {
            Ok(t_.reshape((b, t, self.n_head, hd))?.transpose(1, 2)?.contiguous()?)
        };
        let q = split(&q)?;
        let k = split(&k)?;
        let v = split(&v)?;
        let scale = 1.0 / (hd as f64).sqrt();
        let att = (q.matmul(&k.transpose(2, 3)?)? * scale)?;
        let att = att.broadcast_add(&mask.i((.., .., ..t, ..t))?)?;
        let att = softmax(&att, D::Minus1)?;
        let out = att.matmul(&v)?;
        let out = out.transpose(1, 2)?.contiguous()?.reshape((b, t, c))?;
        Ok(self.proj.forward(&out)?)
    }
}

struct Block {
    ln1: LayerNorm,
    attn: Attn,
    ln2: LayerNorm,
    mlp: Mlp,
}
impl Block {
    fn new(cfg: &Config, vb: VarBuilder) -> Result<Self> {
        Ok(Self {
            ln1: layer_norm(cfg.n_embd, 1e-5, vb.pp("ln1"))?,
            attn: Attn::new(cfg, vb.pp("attn"))?,
            ln2: layer_norm(cfg.n_embd, 1e-5, vb.pp("ln2"))?,
            mlp: Mlp::new(cfg.n_embd, vb.pp("mlp"))?,
        })
    }
    fn forward(&self, x: &Tensor, mask: &Tensor) -> Result<Tensor> {
        let x = (x + self.attn.forward(&self.ln1.forward(x)?, mask)?)?;
        let x = (&x + self.mlp.forward(&self.ln2.forward(&x)?)?)?;
        Ok(x)
    }
}

pub struct Gpt {
    tok: Embedding,
    pos: Embedding,
    blocks: Vec<Block>,
    ln_f: LayerNorm,
    head: Linear,
    mask: Tensor,
    cfg: Config,
    device: Device,
}

impl Gpt {
    pub fn new(cfg: Config, vb: VarBuilder, device: &Device) -> Result<Self> {
        let tok = embedding(cfg.vocab_size, cfg.n_embd, vb.pp("tok"))?;
        let pos = embedding(cfg.block_size, cfg.n_embd, vb.pp("pos"))?;
        let mut blocks = Vec::new();
        for i in 0..cfg.n_layer {
            blocks.push(Block::new(&cfg, vb.pp(format!("block.{i}")))?);
        }
        let ln_f = layer_norm(cfg.n_embd, 1e-5, vb.pp("ln_f"))?;
        let head = linear(cfg.n_embd, cfg.vocab_size, vb.pp("head"))?;
        let mask = causal_mask(cfg.block_size, device)?;
        Ok(Self { tok, pos, blocks, ln_f, head, mask, cfg, device: device.clone() })
    }

    pub fn forward(&self, idx: &Tensor) -> Result<Tensor> {
        let (_b, t) = idx.dims2()?;
        let tok = self.tok.forward(idx)?;
        let pos_ids = Tensor::arange(0u32, t as u32, &self.device)?;
        let pos = self.pos.forward(&pos_ids)?;
        let mut x = tok.broadcast_add(&pos)?;
        for blk in &self.blocks {
            x = blk.forward(&x, &self.mask)?;
        }
        let x = self.ln_f.forward(&x)?;
        Ok(self.head.forward(&x)?)
    }
}

fn causal_mask(t: usize, device: &Device) -> Result<Tensor> {
    let mut data = vec![0f32; t * t];
    for i in 0..t {
        for j in (i + 1)..t {
            data[i * t + j] = f32::NEG_INFINITY;
        }
    }
    Ok(Tensor::from_vec(data, (1, 1, t, t), device)?)
}

// --- training ---------------------------------------------------------------

pub struct TrainReport {
    pub steps: usize,
    pub final_loss: f32,
    pub params: u64,
    pub vocab: usize,
}

pub fn train(text: &str, steps: usize, dir: &Path) -> Result<TrainReport> {
    let device = Device::Cpu;
    let vocab = Vocab::from_text(text, MAX_VOCAB);
    let cfg = Config::small(vocab.len().max(2));
    let data = vocab.encode(text);
    if data.len() < cfg.block_size + 8 {
        return Err(anyhow!("corpus too small to train ({} tokens)", data.len()));
    }

    let varmap = VarMap::new();
    let vb = VarBuilder::from_varmap(&varmap, DType::F32, &device);
    let model = Gpt::new(cfg, vb, &device)?;

    let peak_lr = 3e-3;
    let min_lr = peak_lr * 0.05;
    let warmup = (steps / 25).clamp(1, 120);
    let mut opt = AdamW::new(varmap.all_vars(), ParamsAdamW { lr: peak_lr, ..Default::default() })?;

    let batch = 32usize;
    let block = cfg.block_size;
    let mut rng = rand::thread_rng();
    let mut final_loss = 0f32;

    for step in 0..steps {
        let lr = if step < warmup {
            peak_lr * (step as f64 + 1.0) / warmup as f64
        } else {
            let p = (step - warmup) as f64 / (steps - warmup).max(1) as f64;
            min_lr + 0.5 * (peak_lr - min_lr) * (1.0 + (std::f64::consts::PI * p).cos())
        };
        opt.set_learning_rate(lr);

        let (xb, yb) = batch_from(&data, batch, block, &device, &mut rng)?;
        let logits = model.forward(&xb)?;
        let (b, t, v) = logits.dims3()?;
        let logits = logits.reshape((b * t, v))?;
        let targets = yb.reshape((b * t,))?;
        let loss = cross_entropy(&logits, &targets)?;
        opt.backward_step(&loss)?;
        final_loss = loss.to_scalar::<f32>()?;
        if step % 200 == 0 || step == steps - 1 {
            println!("[queen] step {step}/{steps} lr {lr:.5} loss {final_loss:.4}");
        }
    }

    std::fs::create_dir_all(dir)?;
    varmap.save(dir.join("queen.safetensors"))?;
    std::fs::write(dir.join("queen.json"), serde_json::to_string(&Sidecar { cfg, vocab })?)?;

    Ok(TrainReport { steps, final_loss, params: cfg.n_params(), vocab: cfg.vocab_size })
}

fn batch_from(
    data: &[u32],
    batch: usize,
    block: usize,
    device: &Device,
    rng: &mut impl rand::Rng,
) -> Result<(Tensor, Tensor)> {
    let mut xs = Vec::with_capacity(batch * block);
    let mut ys = Vec::with_capacity(batch * block);
    let max_start = data.len() - block - 1;
    for _ in 0..batch {
        let i = rng.gen_range(0..=max_start);
        xs.extend_from_slice(&data[i..i + block]);
        ys.extend_from_slice(&data[i + 1..i + 1 + block]);
    }
    let x = Tensor::from_vec(xs, (batch, block), device)?;
    let y = Tensor::from_vec(ys, (batch, block), device)?;
    Ok((x, y))
}

// --- generation / loading ---------------------------------------------------

#[derive(Serialize, Deserialize)]
struct Sidecar {
    cfg: Config,
    vocab: Vocab,
}

pub struct ModelInfo {
    pub params: u64,
    pub vocab: usize,
    pub block_size: usize,
}

pub fn exists(dir: &Path) -> bool {
    dir.join("queen.safetensors").exists() && dir.join("queen.json").exists()
}

pub fn model_info(dir: &Path) -> Option<ModelInfo> {
    let s: Sidecar = serde_json::from_str(&std::fs::read_to_string(dir.join("queen.json")).ok()?).ok()?;
    Some(ModelInfo { params: s.cfg.n_params(), vocab: s.cfg.vocab_size, block_size: s.cfg.block_size })
}

pub fn corpus_text(path: &Path) -> std::io::Result<String> {
    let raw = std::fs::read_to_string(path)?;
    let mut text = String::new();
    for line in raw.lines() {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            if let Some(t) = v.get("text").and_then(|t| t.as_str()) {
                text.push_str(t);
                text.push_str(" \n ");
            }
        }
    }
    Ok(text)
}

pub struct Queen {
    model: Gpt,
    vocab: Vocab,
    cfg: Config,
    device: Device,
}

impl Queen {
    pub fn load(dir: &Path) -> Result<Self> {
        let device = Device::Cpu;
        let sidecar: Sidecar =
            serde_json::from_str(&std::fs::read_to_string(dir.join("queen.json"))?)?;
        let mut varmap = VarMap::new();
        let vb = VarBuilder::from_varmap(&varmap, DType::F32, &device);
        let model = Gpt::new(sidecar.cfg, vb, &device)?;
        varmap.load(dir.join("queen.safetensors"))?;
        Ok(Self { model, vocab: sidecar.vocab, cfg: sidecar.cfg, device })
    }

    /// Generate up to `max_new` word tokens continuing `prompt` (top-k sampling).
    pub fn generate(&self, prompt: &str, max_new: usize, temperature: f64) -> Result<String> {
        let mut ids = self.vocab.encode(prompt);
        if ids.is_empty() {
            ids.push(1u32.min(self.vocab.len().saturating_sub(1) as u32));
        }
        let mut rng = rand::thread_rng();
        let temp = temperature.max(1e-3) as f32;
        let penalty = 1.3f32;
        for _ in 0..max_new {
            let start = ids.len().saturating_sub(self.cfg.block_size);
            let ctx = &ids[start..];
            let x = Tensor::from_vec(ctx.to_vec(), (1, ctx.len()), &self.device)?;
            let logits = self.model.forward(&x)?;
            let mut logit = logits.i((0, ctx.len() - 1, ..))?.to_vec1::<f32>()?;

            // repetition penalty: push down tokens seen recently so she does not
            // loop on a period or a digit run
            let recent: std::collections::HashSet<usize> =
                ids.iter().rev().take(48).map(|&t| t as usize).collect();
            for &i in &recent {
                logit[i] = if logit[i] > 0.0 { logit[i] / penalty } else { logit[i] * penalty };
            }
            // never emit the same token three times in a row
            let n = ids.len();
            if n >= 2 && ids[n - 1] == ids[n - 2] {
                logit[ids[n - 1] as usize] = f32::NEG_INFINITY;
            }

            // top-k by (penalised) logit, then a temperature softmax over those k
            let mut idxs: Vec<usize> = (0..logit.len()).collect();
            idxs.sort_by(|&a, &b| logit[b].partial_cmp(&logit[a]).unwrap_or(std::cmp::Ordering::Equal));
            idxs.truncate(TOP_K.min(idxs.len()));
            let maxl = idxs.iter().map(|&i| logit[i]).fold(f32::NEG_INFINITY, f32::max);
            let weights: Vec<f32> = idxs
                .iter()
                .map(|&i| (((logit[i] - maxl) / temp) as f32).exp())
                .collect();
            let dist = WeightedIndex::new(&weights).map_err(|e| anyhow!("sample: {e}"))?;
            let next = idxs[dist.sample(&mut rng)] as u32;
            ids.push(next);
        }
        Ok(self.vocab.decode(&ids).trim().to_string())
    }
}
