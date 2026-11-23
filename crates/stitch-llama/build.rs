//! Builds llama.cpp (the `llama.cpp` submodule) as static libraries with
//! CMake, the multimodal `mtmd` library included, and generates the Rust
//! bindings with bindgen. Metal on Apple (its shaders embedded, so there is
//! no file to ship), CPU elsewhere; iOS and visionOS (devices and
//! simulators) cross-build from macOS.

use std::path::{Path, PathBuf};

fn main() {
  let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
  let src = manifest.join("llama.cpp");
  if !src.join("CMakeLists.txt").exists() {
    panic!("llama.cpp is missing: run `git submodule update --init` in the Stitch checkout");
  }
  let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
  let target = std::env::var("TARGET").unwrap();
  let apple = matches!(os.as_str(), "macos" | "ios" | "visionos");
  let sim = std::env::var("CARGO_CFG_TARGET_ABI").is_ok_and(|a| a == "sim");
  // iOS and visionOS: CMake's name for the system, the SDK, the deployment
  // target (what rustc links for, when set) and clang's target.
  let mobile = match os.as_str() {
    "ios" => Some((
      "iOS",
      if sim { "iphonesimulator" } else { "iphoneos" },
      std::env::var("IPHONEOS_DEPLOYMENT_TARGET").unwrap_or_else(|_| "16.0".into()),
      "ios",
    )),
    "visionos" => Some((
      "visionOS",
      if sim { "xrsimulator" } else { "xros" },
      std::env::var("XROS_DEPLOYMENT_TARGET").unwrap_or_else(|_| "1.0".into()),
      "xros",
    )),
    _ => None,
  };

  let mut cfg = cmake::Config::new(&src);
  // Release even in debug builds: an unoptimized model runs far too slowly.
  cfg.profile("Release");
  for (k, v) in [
    ("BUILD_SHARED_LIBS", "OFF"),
    ("LLAMA_BUILD_COMMON", "OFF"),
    ("LLAMA_BUILD_TESTS", "OFF"),
    ("LLAMA_BUILD_TOOLS", "OFF"),
    ("LLAMA_BUILD_EXAMPLES", "OFF"),
    ("LLAMA_BUILD_SERVER", "OFF"),
    ("LLAMA_BUILD_APP", "OFF"),
    ("LLAMA_BUILD_MTMD", "ON"),
    ("LLAMA_OPENSSL", "OFF"),
    ("LLAMA_SUBPROCESS", "OFF"),
    ("LLAMA_ALL_WARNINGS", "OFF"),
    ("GGML_OPENMP", "OFF"),
    ("MTMD_VIDEO", "OFF"),
  ] {
    cfg.define(k, v);
  }
  if apple {
    cfg.define("GGML_METAL", "ON");
    cfg.define("GGML_METAL_EMBED_LIBRARY", "ON");
    cfg.define("GGML_ACCELERATE", "ON");
    cfg.define("GGML_BLAS", "ON");
  }
  let gpu = if apple { Gpu::None } else { gpu() };
  match &gpu {
    Gpu::Cuda(root) => {
      cfg.define("GGML_CUDA", "ON");
      cfg.define(
        "CMAKE_CUDA_COMPILER",
        root.join("bin/nvcc").display().to_string(),
      );
      // This machine's GPU only: every architecture takes many minutes.
      let arch = std::env::var("STITCH_LLAMA_CUDA_ARCH").unwrap_or_else(|_| "native".into());
      cfg.define("CMAKE_CUDA_ARCHITECTURES", arch);
      if let Ok(cc) = std::env::var("NVCC_CCBIN") {
        cfg.define("CMAKE_CUDA_HOST_COMPILER", cc);
      }
    }
    Gpu::Vulkan => {
      cfg.define("GGML_VULKAN", "ON");
    }
    Gpu::None => {}
  }
  if os == "macos" {
    // What rustc links for, so objects and binary agree.
    let min = std::env::var("MACOSX_DEPLOYMENT_TARGET").unwrap_or_else(|_| "11.0".into());
    cfg.define("CMAKE_OSX_DEPLOYMENT_TARGET", min);
  }
  if let Some((system, sdk_name, min, _)) = &mobile {
    cfg.define("CMAKE_SYSTEM_NAME", system);
    cfg.define("CMAKE_OSX_ARCHITECTURES", "arm64");
    cfg.define("CMAKE_OSX_DEPLOYMENT_TARGET", min);
    cfg.define("GGML_NATIVE", "OFF");
    cfg.define("CMAKE_OSX_SYSROOT", sdk(sdk_name));
  }
  let ninja = which("ninja");
  if ninja {
    cfg.generator("Ninja");
  }
  cfg.build_target("mtmd");
  // Built once per configuration in a cache outside `target/`, shared by
  // every checkout and worktree: a fresh worktree would otherwise rebuild
  // llama.cpp (CUDA: many minutes) since its source path is new.
  let build = match cache_dir(&src, &target, &gpu, ninja) {
    Some(dir) => {
      let build = dir.join("build");
      if !dir.join(DONE).exists() {
        let tmp = dir.with_extension(format!("tmp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        cfg.out_dir(&tmp);
        cfg.build();
        std::fs::write(tmp.join(DONE), "").expect("cache marker written");
        // Another build may have finished first: theirs is as good.
        if std::fs::rename(&tmp, &dir).is_err() {
          let _ = std::fs::remove_dir_all(&tmp);
        }
      }
      build
    }
    None => cfg.build().join("build"),
  };

  // Every static library the build made, in dependency order.
  println!("cargo:rustc-link-search=native={}", build.display());
  let mut dirs = Vec::new();
  libraries(&build, &mut dirs);
  dirs.sort();
  dirs.dedup();
  for d in &dirs {
    println!("cargo:rustc-link-search=native={}", d.display());
  }
  // The known ones in dependency order, then whatever else the build
  // made (vendored helpers), so a static link resolves on every platform.
  let mut found: Vec<String> = Vec::new();
  for d in &dirs {
    for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
      let name = e.file_name().to_string_lossy().into_owned();
      if let Some(lib) = name.strip_prefix("lib").and_then(|n| n.strip_suffix(".a")) {
        found.push(lib.to_owned());
      }
    }
  }
  let order = [
    "mtmd",
    "llama",
    "ggml",
    "ggml-cpu",
    "ggml-metal",
    "ggml-cuda",
    "ggml-vulkan",
    "ggml-blas",
    "ggml-base",
  ];
  let mut libs: Vec<String> = order
    .iter()
    .filter(|l| found.iter().any(|f| f == *l))
    .map(|l| (*l).to_owned())
    .collect();
  libs.extend(found.into_iter().filter(|f| !order.contains(&f.as_str())));
  libs.dedup();
  for lib in libs {
    println!("cargo:rustc-link-lib=static={lib}");
  }
  if apple {
    for fw in ["Foundation", "Metal", "MetalKit", "Accelerate"] {
      println!("cargo:rustc-link-lib=framework={fw}");
    }
    println!("cargo:rustc-link-lib=c++");
  } else {
    println!("cargo:rustc-link-lib=stdc++");
    println!("cargo:rustc-link-lib=m");
  }
  match &gpu {
    Gpu::Cuda(root) => {
      // The toolkit's shared libraries, found again at run time (its
      // directory is in the binary's run path), and the driver's.
      for dir in ["lib64", "lib64/stubs", "targets/x86_64-linux/lib"] {
        let d = root.join(dir);
        if d.is_dir() {
          println!("cargo:rustc-link-search=native={}", d.display());
          if !dir.ends_with("stubs") {
            println!("cargo:rustc-link-arg=-Wl,-rpath,{}", d.display());
          }
        }
      }
      for lib in ["cublas", "cublasLt", "cudart", "cuda"] {
        println!("cargo:rustc-link-lib=dylib={lib}");
      }
    }
    Gpu::Vulkan => println!("cargo:rustc-link-lib=dylib=vulkan"),
    Gpu::None => {}
  }
  println!("cargo:rerun-if-env-changed=STITCH_LLAMA_GPU");
  println!("cargo:rerun-if-env-changed=STITCH_LLAMA_CUDA_ARCH");
  println!("cargo:rerun-if-env-changed=CUDA_PATH");
  println!("cargo:rerun-if-env-changed=STITCH_LLAMA_CACHE");

  let mut bindings = bindgen::Builder::default()
    .header(manifest.join("wrapper.h").display().to_string())
    .clang_arg(format!("-I{}", src.join("include").display()))
    .clang_arg(format!("-I{}", src.join("ggml/include").display()))
    .clang_arg(format!("-I{}", src.join("tools/mtmd").display()))
    .allowlist_function("llama_.*|mtmd_.*|ggml_log_.*")
    .allowlist_type("llama_.*|mtmd_.*|ggml_log_.*")
    .allowlist_var("LLAMA_.*|MTMD_.*")
    .prepend_enum_name(false)
    .layout_tests(false)
    .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()));
  if let Some((_, sdk_name, min, clang_os)) = &mobile {
    let env = if sim { "-simulator" } else { "" };
    bindings = bindings
      .clang_arg(format!("--target=arm64-apple-{clang_os}{min}{env}"))
      .clang_arg(format!("-isysroot{}", sdk(sdk_name)));
  } else if target.contains("apple") {
    bindings = bindings.clang_arg(format!("-isysroot{}", sdk("macosx")));
  }
  bindings
    .generate()
    .expect("llama.cpp bindings")
    .write_to_file(PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("bindings.rs"))
    .expect("bindings written");
  println!("cargo:rerun-if-changed=wrapper.h");
  println!("cargo:rerun-if-changed=build.rs");
}

/// The GPU backend built in beside the CPU's, off Apple (Metal there).
enum Gpu {
  None,
  /// CUDA, from the toolkit at this root.
  Cuda(PathBuf),
  Vulkan,
}

/// `STITCH_LLAMA_GPU` decides (`cuda`, `vulkan`, `off`); unset, CUDA when
/// the build machine has the toolkit (`CUDA_PATH`, `/opt/cuda`,
/// `/usr/local/cuda`, or `nvcc` on the path) and an NVIDIA GPU, else none:
/// the CPU path, which a node without that GPU could not run otherwise
/// (the binary links the CUDA libraries).
fn gpu() -> Gpu {
  let toolkit = || {
    let mut roots: Vec<PathBuf> = std::env::var_os("CUDA_PATH")
      .map(PathBuf::from)
      .into_iter()
      .collect();
    roots.extend(["/opt/cuda", "/usr/local/cuda"].map(PathBuf::from));
    if let Ok(out) = std::process::Command::new("which").arg("nvcc").output()
      && out.status.success()
    {
      let p = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
      if let Some(root) = p.parent().and_then(Path::parent) {
        roots.push(root.to_owned());
      }
    }
    roots.into_iter().find(|r| r.join("bin/nvcc").is_file())
  };
  match std::env::var("STITCH_LLAMA_GPU").ok().as_deref() {
    Some("off" | "none" | "cpu") => Gpu::None,
    Some("vulkan") => Gpu::Vulkan,
    Some("cuda") => {
      Gpu::Cuda(toolkit().expect("STITCH_LLAMA_GPU=cuda: no CUDA toolkit (set CUDA_PATH)"))
    }
    Some(other) => panic!("STITCH_LLAMA_GPU={other}: cuda, vulkan or off"),
    None => match toolkit() {
      Some(root) if Path::new("/dev/nvidia0").exists() => Gpu::Cuda(root),
      _ => Gpu::None,
    },
  }
}

/// Marks a complete build in a cache directory.
const DONE: &str = ".stitch-built";

/// Where this configuration's llama.cpp build is cached:
/// `$STITCH_LLAMA_CACHE` (`off` disables it), else `~/.cache/stitch/llama`,
/// one directory per submodule commit and everything that changes the
/// objects. None, so built in `OUT_DIR` as before, when the submodule has
/// local changes or its commit cannot be read.
fn cache_dir(src: &Path, target: &str, gpu: &Gpu, ninja: bool) -> Option<PathBuf> {
  use std::hash::{Hash, Hasher};
  let root = match std::env::var_os("STITCH_LLAMA_CACHE") {
    Some(v) if v == "off" => return None,
    Some(v) => PathBuf::from(v),
    None => std::env::var_os("XDG_CACHE_HOME")
      .map(PathBuf::from)
      .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?
      .join("stitch/llama"),
  };
  let git = |args: &[&str]| {
    let out = std::process::Command::new("git")
      .arg("-C")
      .arg(src)
      .args(args)
      .output()
      .ok()?;
    out
      .status
      .success()
      .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
  };
  let commit = git(&["rev-parse", "HEAD"])?;
  if !git(&["status", "--porcelain", "--untracked-files=no"])?.is_empty() {
    return None;
  }
  let gpu = match gpu {
    Gpu::None => "cpu".to_owned(),
    Gpu::Cuda(root) => format!("cuda:{}", root.display()),
    Gpu::Vulkan => "vulkan".to_owned(),
  };
  let mut h = std::collections::hash_map::DefaultHasher::new();
  (target, gpu, ninja, include_str!("build.rs")).hash(&mut h);
  for var in [
    "STITCH_LLAMA_CUDA_ARCH",
    "NVCC_CCBIN",
    "MACOSX_DEPLOYMENT_TARGET",
    "IPHONEOS_DEPLOYMENT_TARGET",
    "XROS_DEPLOYMENT_TARGET",
    "CC",
    "CXX",
    "CFLAGS",
    "CXXFLAGS",
    "OPT_LEVEL",
    "DEBUG",
  ] {
    (var, std::env::var(var).ok()).hash(&mut h);
  }
  let dir = root.join(format!("{}-{:016x}", &commit[..12], h.finish()));
  std::fs::create_dir_all(&root).ok()?;
  Some(dir)
}

/// Directories under `dir` holding static libraries.
fn libraries(dir: &Path, out: &mut Vec<PathBuf>) {
  let Ok(entries) = std::fs::read_dir(dir) else {
    return;
  };
  for e in entries.flatten() {
    let p = e.path();
    if p.is_dir() {
      libraries(&p, out);
    } else if p.extension().is_some_and(|x| x == "a") {
      out.push(dir.to_owned());
    }
  }
}

fn sdk(name: &str) -> String {
  let out = std::process::Command::new("xcrun")
    .args(["--sdk", name, "--show-sdk-path"])
    .output()
    .expect("xcrun finds the SDK");
  String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

fn which(tool: &str) -> bool {
  std::process::Command::new(tool)
    .arg("--version")
    .output()
    .is_ok()
}
