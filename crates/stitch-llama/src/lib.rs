//! llama.cpp in-process, for embeddings: text, and images through the
//! model's vision projector (`mtmd`). Built for EmbeddingGemma 2 (one
//! space for text and images, mean-pooled, L2-normalized), and for any
//! embedding GGUF llama.cpp reads.
//!
//! An [`Embedder`] owns a model and its context; it is `Send` but not
//! `Sync`: one embedding at a time, behind a lock where shared. llama.cpp's
//! log goes nowhere unless `STITCH_LLAMA_LOG` is set.

#![allow(unsafe_code)]

use std::ffi::{CString, c_char, c_void};
use std::path::Path;
use std::ptr;
use std::sync::Once;

#[allow(
  non_upper_case_globals,
  non_camel_case_types,
  non_snake_case,
  dead_code,
  clippy::all
)]
pub mod sys {
  include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
  #[error("could not load {0}")]
  Load(String),
  #[error("the text is empty")]
  Empty,
  #[error("tokenizing failed")]
  Tokenize,
  #[error("this embedder has no vision projector")]
  NoVision,
  #[error("the image could not be read")]
  Image,
  #[error("llama.cpp failed: {0}")]
  Process(&'static str),
}

pub type Result<T> = std::result::Result<T, Error>;

static INIT: Once = Once::new();

unsafe extern "C" fn quiet(_level: sys::ggml_log_level, text: *const c_char, _: *mut c_void) {
  if std::env::var_os("STITCH_LLAMA_LOG").is_some() && !text.is_null() {
    // SAFETY: llama.cpp passes a NUL-terminated message.
    let s = unsafe { std::ffi::CStr::from_ptr(text) };
    eprint!("{}", s.to_string_lossy());
  }
}

fn init() {
  INIT.call_once(|| unsafe {
    sys::llama_log_set(Some(quiet), ptr::null_mut());
    sys::mtmd_log_set(Some(quiet), ptr::null_mut());
    sys::mtmd_helper_log_set(Some(quiet), ptr::null_mut());
    sys::llama_backend_init();
  });
}

/// CPU threads for a context: half the logical CPUs (the physical cores,
/// near enough), at most 8. ggml's workers spin while they wait, so every
/// CPU of a 32-thread machine sat at 100% through each batch, and a small
/// embedding gains little past a handful of cores.
fn threads() -> i32 {
  let n = std::thread::available_parallelism()
    .map(|n| n.get())
    .unwrap_or(4);
  (n / 2).clamp(1, 8) as i32
}

fn cstring(p: &Path) -> Result<CString> {
  CString::new(p.to_string_lossy().as_bytes()).map_err(|_| Error::Load(p.display().to_string()))
}

/// The longest side of an image the vision projector gets: larger ones are
/// scaled down first. EmbeddingGemma 2's projector (gemma4v: 16 px patches
/// pooled 3×3, so 48 px a token) takes 70 to 1120 tokens of any aspect, and
/// its vision graph grows with the square of the patches: a 1206×2622
/// phone screenshot is about 1100 tokens, ten thousand patches, a compute
/// buffer an iPhone may refuse (a refused Metal buffer then crashed in
/// ggml's allocator). 768 px is the projector's own warmup size (256
/// tokens, 16×16), so no image needs more than that.
pub const IMAGE_SIDE: u32 = 768;

/// The most tokens one image becomes: `IMAGE_SIDE` squared over 48².
pub const IMAGE_MAX_TOKENS: i32 = 256;

/// What to load.
#[derive(Clone, Debug)]
pub struct Options {
  /// Longest input, in tokens (EmbeddingGemma 2 takes 8192).
  pub context: u32,
  /// Use the GPU (Metal) where there is one.
  pub gpu: bool,
}

impl Default for Options {
  fn default() -> Options {
    Options {
      context: 8192,
      gpu: true,
    }
  }
}

pub struct Embedder {
  model: *mut sys::llama_model,
  ctx: *mut sys::llama_context,
  vision: *mut sys::mtmd_context,
  n_embd: usize,
  n_embd_inp: usize,
  n_ctx: usize,
}

// SAFETY: the pointers are owned by this value and used through `&mut self`
// only; llama.cpp contexts may move between threads, not be shared.
unsafe impl Send for Embedder {}

impl Drop for Embedder {
  fn drop(&mut self) {
    unsafe {
      if !self.vision.is_null() {
        sys::mtmd_free(self.vision);
      }
      if !self.ctx.is_null() {
        sys::llama_free(self.ctx);
      }
      if !self.model.is_null() {
        sys::llama_model_free(self.model);
      }
    }
  }
}

impl Embedder {
  /// Loads an embedding model, and its vision projector when given.
  pub fn load(model: &Path, projector: Option<&Path>, opts: &Options) -> Result<Embedder> {
    init();
    let path = cstring(model)?;
    unsafe {
      let mut mp = sys::llama_model_default_params();
      if !opts.gpu {
        mp.n_gpu_layers = 0;
      }
      let m = sys::llama_model_load_from_file(path.as_ptr(), mp);
      if m.is_null() {
        return Err(Error::Load(model.display().to_string()));
      }
      let mut cp = sys::llama_context_default_params();
      cp.embeddings = true;
      cp.n_ctx = opts.context;
      // A non-causal model sees a whole input in one pass.
      cp.n_batch = opts.context;
      cp.n_ubatch = opts.context;
      cp.n_seq_max = 1;
      let threads = threads();
      cp.n_threads = threads;
      cp.n_threads_batch = threads;
      let ctx = sys::llama_init_from_model(m, cp);
      let mut out = Embedder {
        model: m,
        ctx,
        vision: ptr::null_mut(),
        n_embd: sys::llama_model_n_embd_out(m).max(0) as usize,
        n_embd_inp: sys::llama_model_n_embd_inp(m).max(0) as usize,
        n_ctx: opts.context as usize,
      };
      if ctx.is_null() {
        return Err(Error::Load(format!("a context for {}", model.display())));
      }
      if let Some(p) = projector {
        let proj = cstring(p)?;
        let mut vp = sys::mtmd_context_params_default();
        vp.use_gpu = opts.gpu;
        vp.print_timings = false;
        vp.n_threads = threads;
        vp.warmup = false;
        // Bounds the vision graph whatever image reaches the projector
        // (`embed_image` scales down first; this is the backstop).
        vp.image_max_tokens = IMAGE_MAX_TOKENS;
        out.vision = sys::mtmd_init_from_file(proj.as_ptr(), m, vp);
        if out.vision.is_null() {
          return Err(Error::Load(p.display().to_string()));
        }
      }
      Ok(out)
    }
  }

  /// The embedding's width (768 for EmbeddingGemma 2).
  pub fn dimensions(&self) -> usize {
    self.n_embd
  }

  /// Whether images can be embedded.
  pub fn sees(&self) -> bool {
    !self.vision.is_null() && unsafe { sys::mtmd_support_vision(self.vision) }
  }

  fn tokenize(&self, text: &str) -> Result<Vec<sys::llama_token>> {
    unsafe {
      let vocab = sys::llama_model_get_vocab(self.model);
      let mut tokens = vec![0 as sys::llama_token; text.len() + 8];
      let mut n = sys::llama_tokenize(
        vocab,
        text.as_ptr().cast(),
        text.len() as i32,
        tokens.as_mut_ptr(),
        tokens.len() as i32,
        true,
        false,
      );
      if n < 0 {
        tokens.resize((-n) as usize, 0);
        n = sys::llama_tokenize(
          vocab,
          text.as_ptr().cast(),
          text.len() as i32,
          tokens.as_mut_ptr(),
          tokens.len() as i32,
          true,
          false,
        );
      }
      if n < 0 {
        return Err(Error::Tokenize);
      }
      tokens.truncate(n as usize);
      Ok(tokens)
    }
  }

  /// One text's embedding, unit length. Text past the context is cut.
  pub fn embed(&mut self, text: &str) -> Result<Vec<f32>> {
    if text.trim().is_empty() {
      return Err(Error::Empty);
    }
    let mut tokens = self.tokenize(text)?;
    if tokens.is_empty() {
      return Err(Error::Empty);
    }
    tokens.truncate(self.n_ctx);
    let inputs: Vec<Input<'_>> = tokens.iter().map(|&t| Input::Token(t)).collect();
    self.run(&inputs)
  }

  /// An image's embedding (PNG, JPEG, BMP, GIF bytes), in the same space as
  /// text; `caption` is text around it, `None` for the image alone.
  pub fn embed_image(&mut self, image: &[u8], caption: Option<&str>) -> Result<Vec<f32>> {
    if !self.sees() {
      return Err(Error::NoVision);
    }
    unsafe {
      let marker = std::ffi::CStr::from_ptr(sys::mtmd_default_marker())
        .to_string_lossy()
        .into_owned();
      let prompt = match caption {
        Some(c) => format!("{marker} {c}"),
        None => marker,
      };
      let raw = match fit_image(image) {
        Some(rgb) => sys::mtmd_bitmap_init(rgb.width(), rgb.height(), rgb.as_raw().as_ptr()),
        // What `image` cannot read, llama.cpp's own decoder may
        // (`IMAGE_MAX_TOKENS` still bounds it).
        None => {
          sys::mtmd_helper_bitmap_init_from_buf(
            self.vision,
            image.as_ptr(),
            image.len(),
            false,
            sys::mtmd_helper_init_opt_default(),
          )
          .bitmap
        }
      };
      if raw.is_null() {
        return Err(Error::Image);
      }
      let bitmap = Bitmap(raw);
      let chunks = Chunks(sys::mtmd_input_chunks_init());
      let text = CString::new(prompt).map_err(|_| Error::Image)?;
      let input = sys::mtmd_input_text {
        text: text.as_ptr(),
        text_len: text.as_bytes().len(),
        add_special: true,
        parse_special: true,
      };
      let bitmaps = [bitmap.0 as *const sys::mtmd_bitmap];
      if sys::mtmd_tokenize(self.vision, chunks.0, &input, bitmaps.as_ptr(), 1) != 0 {
        return Err(Error::Image);
      }
      // Text tokens as they are; media chunks encoded by the projector.
      let mut owned: Vec<Vec<f32>> = Vec::new();
      let mut plan: Vec<(bool, usize, usize)> = Vec::new();
      let mut text_tokens: Vec<sys::llama_token> = Vec::new();
      for i in 0..sys::mtmd_input_chunks_size(chunks.0) {
        let chunk = sys::mtmd_input_chunks_get(chunks.0, i);
        if sys::mtmd_input_chunk_get_type(chunk) == sys::MTMD_INPUT_CHUNK_TYPE_TEXT {
          let mut n = 0usize;
          let toks = sys::mtmd_input_chunk_get_tokens_text(chunk, &mut n);
          let start = text_tokens.len();
          text_tokens.extend_from_slice(std::slice::from_raw_parts(toks, n));
          plan.push((false, start, n));
        } else {
          if sys::mtmd_encode_chunk(self.vision, chunk) != 0 {
            return Err(Error::Process("encoding the image"));
          }
          let n = sys::mtmd_input_chunk_get_n_tokens(chunk);
          let embd = sys::mtmd_get_output_embd(self.vision);
          owned.push(std::slice::from_raw_parts(embd, n * self.n_embd_inp).to_vec());
          plan.push((true, owned.len() - 1, n));
        }
      }
      let mut inputs: Vec<Input<'_>> = Vec::new();
      for (media, at, n) in plan {
        if media {
          let rows = &owned[at];
          inputs.extend(
            (0..n).map(|r| Input::Embd(&rows[r * self.n_embd_inp..(r + 1) * self.n_embd_inp])),
          );
        } else {
          inputs.extend(text_tokens[at..at + n].iter().map(|&t| Input::Token(t)));
        }
      }
      if inputs.len() > self.n_ctx {
        return Err(Error::Process("the image takes more than the context"));
      }
      self.run(&inputs)
    }
  }

  /// Runs one sequence and returns its pooled, normalized embedding.
  fn run(&mut self, inputs: &[Input<'_>]) -> Result<Vec<f32>> {
    unsafe {
      let mem = sys::llama_get_memory(self.ctx);
      if !mem.is_null() {
        sys::llama_memory_clear(mem, true);
      }
      let batch = Batch(sys::llama_batch_ext_init(self.ctx));
      for (pos, input) in inputs.iter().enumerate() {
        let idx = match input {
          Input::Token(t) => sys::llama_batch_ext_add_token(batch.0, 0, *t),
          Input::Embd(row) => sys::llama_batch_ext_add_embd(
            batch.0,
            0,
            sys::llama_embd {
              data: row.as_ptr(),
              n_rows: 1,
              n_embd: row.len(),
            },
          ),
        };
        if idx < 0 {
          return Err(Error::Process("adding to the batch"));
        }
        let p = pos as sys::llama_pos;
        if !sys::llama_batch_ext_set_pos(batch.0, idx, &p) {
          return Err(Error::Process("setting a position"));
        }
        sys::llama_batch_ext_set_output_embd(batch.0, idx, true);
      }
      if sys::llama_process(self.ctx, sys::LLAMA_PROCESS_TYPE_DECODE, batch.0) != 0 {
        return Err(Error::Process("running the model"));
      }
      let mut out = {
        let seq = sys::llama_get_embeddings_seq(self.ctx, 0);
        if seq.is_null() {
          return Err(Error::Process("reading the embedding"));
        }
        std::slice::from_raw_parts(seq, self.n_embd).to_vec()
      };
      normalize(&mut out);
      if out.iter().any(|x| !x.is_finite()) {
        return Err(Error::Process("the embedding is not finite"));
      }
      Ok(out)
    }
  }
}

enum Input<'a> {
  Token(sys::llama_token),
  Embd(&'a [f32]),
}

struct Batch(*mut sys::llama_batch_ext);
impl Drop for Batch {
  fn drop(&mut self) {
    unsafe { sys::llama_batch_ext_free(self.0) }
  }
}

struct Chunks(*mut sys::mtmd_input_chunks);
impl Drop for Chunks {
  fn drop(&mut self) {
    unsafe { sys::mtmd_input_chunks_free(self.0) }
  }
}

struct Bitmap(*mut sys::mtmd_bitmap);
impl Drop for Bitmap {
  fn drop(&mut self) {
    unsafe { sys::mtmd_bitmap_free(self.0) }
  }
}

#[cfg(target_os = "ios")]
unsafe extern "C" {
  /// <os/proc.h>: how much more this process may allocate before iOS
  /// terminates it.
  fn os_proc_available_memory() -> usize;
}

/// How much more memory this process may take, where the OS says (iOS,
/// whose limit is per app); `None` elsewhere.
pub fn available_memory() -> Option<u64> {
  #[cfg(target_os = "ios")]
  {
    // SAFETY: no arguments, no state; available since iOS 13.
    Some(unsafe { os_proc_available_memory() } as u64)
  }
  #[cfg(not(target_os = "ios"))]
  {
    None
  }
}

/// An image's pixels as the vision projector gets them: decoded (PNG,
/// JPEG, GIF's first frame, BMP), RGB, and scaled down to `IMAGE_SIDE` on
/// its long side when larger; `None` when it cannot be read.
pub fn fit_image(bytes: &[u8]) -> Option<image::RgbImage> {
  let img = image::load_from_memory(bytes).ok()?;
  let (w, h) = (img.width(), img.height());
  if w == 0 || h == 0 {
    return None;
  }
  let img = if w.max(h) > IMAGE_SIDE {
    img.resize(
      IMAGE_SIDE,
      IMAGE_SIDE,
      image::imageops::FilterType::CatmullRom,
    )
  } else {
    img
  };
  Some(img.into_rgb8())
}

/// Scales `v` to unit length.
pub fn normalize(v: &mut [f32]) {
  let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
  if norm > 0.0 {
    v.iter_mut().for_each(|x| *x /= norm);
  }
}

/// Cosine similarity of unit vectors.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
  a.iter().zip(b).map(|(x, y)| x * y).sum()
}
