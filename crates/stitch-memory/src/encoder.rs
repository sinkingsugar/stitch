//! The encoder: EmbeddingGemma 2 in-process, loaded on first use. Every
//! vector it hands out is int8 round-tripped, so the values a node gates on
//! are the ones it stores and every other node folds.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use stitch_llama::{Embedder, Options};

use crate::{DOC_PREFIX, QUERY_PREFIX, pack, unpack};

/// The model files, in a node's `models/` directory.
#[derive(Clone, Debug)]
pub struct ModelFiles {
  pub model: PathBuf,
  /// The vision projector, for images.
  pub projector: Option<PathBuf>,
}

impl ModelFiles {
  pub const MODEL: &'static str = "embeddinggemma-2-Q8_0.gguf";
  pub const PROJECTOR: &'static str = "mmproj-embeddinggemma-2-Q8_0.gguf";
  /// Where they are published (ggml-org's official conversion).
  pub const SOURCE: &'static str =
    "https://huggingface.co/ggml-org/embeddinggemma-2-GGUF/resolve/main";

  /// The files in `dir`, when the model is there.
  pub fn in_dir(dir: &Path) -> Option<ModelFiles> {
    let model = dir.join(Self::MODEL);
    let projector = dir.join(Self::PROJECTOR);
    model.is_file().then(|| ModelFiles {
      model,
      projector: projector.is_file().then_some(projector),
    })
  }
}

pub struct Encoder {
  files: ModelFiles,
  images: bool,
  gpu: bool,
  context: u32,
  inner: Mutex<Option<Embedder>>,
  /// Unloaded and refusing (an app in the background, where the GPU is
  /// not allowed).
  suspended: AtomicBool,
}

/// What a suspended encoder answers.
const SUSPENDED: stitch_llama::Error = stitch_llama::Error::Process("the encoder is suspended");

impl Encoder {
  /// `images`: load the vision projector too (about 550 MB more memory).
  /// `context`: the longest input, in tokens (longer is cut); the compute
  /// buffers grow with it (about 1.5 GB at 8192, 280 MB at 2048).
  pub fn new(files: ModelFiles, images: bool, gpu: bool, context: u32) -> Encoder {
    Encoder {
      files,
      images,
      gpu,
      context,
      inner: Mutex::new(None),
      suspended: AtomicBool::new(false),
    }
  }

  /// Suspends the encoder (unloaded, every call refused until resumed) or
  /// resumes it (loaded again on the next use). An iOS app in the
  /// background may not use the GPU, and llama.cpp's Metal backend fails
  /// for good once refused.
  pub fn suspend(&self, on: bool) {
    let mut guard = self.inner.lock().expect("encoder lock");
    self.suspended.store(on, Ordering::SeqCst);
    if on {
      *guard = None;
    }
  }

  pub fn suspended(&self) -> bool {
    self.suspended.load(Ordering::SeqCst)
  }

  /// Whether images can be embedded.
  pub fn sees(&self) -> bool {
    self.images && self.files.projector.is_some()
  }

  fn with<T>(
    &self,
    f: impl FnOnce(&mut Embedder) -> stitch_llama::Result<T>,
  ) -> stitch_llama::Result<T> {
    let mut guard = self.inner.lock().expect("encoder lock");
    if self.suspended() {
      return Err(SUSPENDED);
    }
    if guard.is_none() {
      let projector = self.files.projector.as_deref().filter(|_| self.images);
      *guard = Some(Embedder::load(
        &self.files.model,
        projector,
        &Options {
          gpu: self.gpu,
          context: self.context,
        },
      )?);
    }
    let out = f(guard.as_mut().expect("loaded"));
    // A failed computation can leave the backend failing for good (Metal's,
    // after a command buffer the OS refused): loaded again on the next use.
    if matches!(
      out,
      Err(stitch_llama::Error::Process(
        "running the model" | "encoding the image"
      ))
    ) {
      *guard = None;
    }
    out
  }

  fn round_trip(v: Vec<f32>) -> Vec<f32> {
    unpack(&pack(&v))
  }

  /// Stored content (a message), int8 round-tripped.
  pub fn document(&self, text: &str) -> stitch_llama::Result<Vec<f32>> {
    self
      .with(|e| e.embed(&format!("{DOC_PREFIX}{text}")))
      .map(Self::round_trip)
  }

  /// A recall cue, int8 round-tripped.
  pub fn query(&self, text: &str) -> stitch_llama::Result<Vec<f32>> {
    self
      .with(|e| e.embed(&format!("{QUERY_PREFIX}{text}")))
      .map(Self::round_trip)
  }

  /// An image (PNG, JPEG, GIF, BMP bytes), int8 round-tripped.
  pub fn image(&self, bytes: &[u8]) -> stitch_llama::Result<Vec<f32>> {
    if !self.sees() {
      return Err(stitch_llama::Error::NoVision);
    }
    self
      .with(|e| e.embed_image(bytes, None))
      .map(Self::round_trip)
  }

  /// Frees the model's memory; it loads again on the next use.
  pub fn unload(&self) {
    *self.inner.lock().expect("encoder lock") = None;
  }
}
