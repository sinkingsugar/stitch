//! The content-addressed blob store: `<dir>/blake3/9f/2c/<hash>`
//! (docs/records.md §7). Outside git, one per node.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use stitch_records::BlobHash;

use crate::error::IoContext;
use crate::{Error, Result};

#[derive(Clone, Debug)]
pub struct BlobStore {
  dir: PathBuf,
}

impl BlobStore {
  pub fn open(dir: &Path) -> Result<Self> {
    let tmp = dir.join("tmp");
    fs::create_dir_all(&tmp).at(&tmp)?;
    Ok(BlobStore {
      dir: dir.to_owned(),
    })
  }

  pub fn path(&self, hash: &BlobHash) -> PathBuf {
    let hex = hash.hex();
    self
      .dir
      .join("blake3")
      .join(&hex[..2])
      .join(&hex[2..4])
      .join(hex)
  }

  pub fn contains(&self, hash: &BlobHash) -> bool {
    self.path(hash).is_file()
  }

  pub fn put_bytes(&self, bytes: &[u8]) -> Result<BlobHash> {
    Ok(self.put_reader(bytes)?.0)
  }

  pub fn put_file(&self, path: &Path) -> Result<(BlobHash, u64)> {
    let f = fs::File::open(path).at(path)?;
    self.put_reader(f)
  }

  /// Streams `reader` into the store, hashing as it goes. Returns the hash
  /// and the size.
  pub fn put_reader(&self, mut reader: impl Read) -> Result<(BlobHash, u64)> {
    let mut w = self.writer()?;
    let mut buf = vec![0u8; 1 << 16];
    loop {
      let n = match reader.read(&mut buf) {
        Ok(0) => break,
        Ok(n) => n,
        Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
        Err(e) => return Err(e).at(&w.tmp),
      };
      w.write(&buf[..n])?;
    }
    w.finish(None)
  }

  /// A blob written in pieces, for content that arrives over the network.
  pub fn writer(&self) -> Result<BlobWriter> {
    let tmp = self
      .dir
      .join("tmp")
      .join(format!("{:016x}", rand::random::<u64>()));
    let file = fs::File::create(&tmp).at(&tmp)?;
    Ok(BlobWriter {
      store: self.clone(),
      tmp,
      file: Some(file),
      hasher: blake3::Hasher::new(),
      size: 0,
      keep: false,
    })
  }

  /// A blob received in pieces that may be cut off: what came so far is
  /// kept (`tmp/partial-<hash>`) and the next writer for the same hash goes
  /// on from there (`BlobWriter::size` says from where). Dropped without
  /// `finish`, it stays for that next one; a finish that fails the hash
  /// removes it.
  pub fn resume(&self, hash: &BlobHash) -> Result<BlobWriter> {
    let tmp = self.dir.join("tmp").join(format!("partial-{}", hash.hex()));
    let mut hasher = blake3::Hasher::new();
    let size = match fs::File::open(&tmp) {
      Ok(f) => {
        hasher.update_reader(f).at(&tmp)?;
        fs::metadata(&tmp).at(&tmp)?.len()
      }
      Err(_) => 0,
    };
    let file = fs::OpenOptions::new()
      .create(true)
      .append(true)
      .open(&tmp)
      .at(&tmp)?;
    Ok(BlobWriter {
      store: self.clone(),
      tmp,
      file: Some(file),
      hasher,
      size,
      keep: true,
    })
  }

  /// Starts a resumable blob over: what was kept of it is dropped.
  pub fn drop_partial(&self, hash: &BlobHash) {
    let _ = fs::remove_file(self.dir.join("tmp").join(format!("partial-{}", hash.hex())));
  }

  pub fn open_blob(&self, hash: &BlobHash) -> Result<fs::File> {
    let p = self.path(hash);
    fs::File::open(&p).at(&p)
  }

  /// Rehashes a stored blob; false if its content no longer matches.
  pub fn verify(&self, hash: &BlobHash) -> Result<bool> {
    let p = self.path(hash);
    let mut hasher = blake3::Hasher::new();
    hasher.update_reader(fs::File::open(&p).at(&p)?).at(&p)?;
    Ok(hasher.finalize() == hash.0)
  }
}

/// A blob being written. Dropped without `finish`, it leaves nothing behind.
pub struct BlobWriter {
  store: BlobStore,
  tmp: PathBuf,
  file: Option<fs::File>,
  hasher: blake3::Hasher,
  size: u64,
  /// A resumable one: kept when dropped unfinished.
  keep: bool,
}

impl BlobWriter {
  pub fn write(&mut self, bytes: &[u8]) -> Result<()> {
    self.hasher.update(bytes);
    self.size += bytes.len() as u64;
    let file = self.file.as_mut().expect("open until finished");
    file.write_all(bytes).at(&self.tmp)
  }

  pub fn size(&self) -> u64 {
    self.size
  }

  /// Syncs the content and moves it into place under its hash. With
  /// `expected`, content that hashes otherwise is discarded and refused.
  pub fn finish(mut self, expected: Option<&BlobHash>) -> Result<(BlobHash, u64)> {
    let file = self.file.take().expect("open until finished");
    let hash = BlobHash(self.hasher.finalize());
    if let Some(e) = expected
      && *e != hash
    {
      // Kept or not, content that hashes otherwise is never gone on from.
      self.keep = false;
      return Err(Error::Mismatch {
        path: e.to_string(),
        message: format!("content hashes to {hash}"),
      });
    }
    file.sync_all().at(&self.tmp)?;
    drop(file);
    let target = self.store.path(&hash);
    if !target.is_file() {
      let parent = target.parent().expect("blob paths have a parent");
      fs::create_dir_all(parent).at(parent)?;
      fs::rename(&self.tmp, &target).at(&target)?;
    }
    self.keep = false;
    Ok((hash, self.size))
  }
}

impl Drop for BlobWriter {
  fn drop(&mut self) {
    // After a successful finish the temporary file was renamed away (or
    // the blob was there already: it goes).
    if !self.keep {
      let _ = fs::remove_file(&self.tmp);
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn blobs_are_stored_once_by_hash() {
    let dir = tempfile::tempdir().unwrap();
    let store = BlobStore::open(dir.path()).unwrap();
    let data = vec![7u8; 200_000];
    let h = store.put_bytes(&data).unwrap();
    assert_eq!(h, BlobHash::of(&data));
    assert_eq!(store.put_bytes(&data).unwrap(), h);
    assert!(store.contains(&h));
    assert!(store.verify(&h).unwrap());
    let mut back = Vec::new();
    store.open_blob(&h).unwrap().read_to_end(&mut back).unwrap();
    assert_eq!(back, data);
    assert!(store.path(&h).starts_with(dir.path().join("blake3")));
    assert_eq!(fs::read_dir(dir.path().join("tmp")).unwrap().count(), 0);
  }

  #[test]
  fn a_resumed_blob_goes_on_from_what_came() {
    let dir = tempfile::tempdir().unwrap();
    let store = BlobStore::open(dir.path()).unwrap();
    let data = vec![9u8; 150_000];
    let hash = BlobHash::of(&data);
    let mut w = store.resume(&hash).unwrap();
    assert_eq!(w.size(), 0);
    w.write(&data[..60_000]).unwrap();
    drop(w);
    let mut w = store.resume(&hash).unwrap();
    assert_eq!(w.size(), 60_000);
    w.write(&data[60_000..]).unwrap();
    assert_eq!(w.finish(Some(&hash)).unwrap(), (hash, 150_000));
    assert!(store.verify(&hash).unwrap());
    assert_eq!(fs::read_dir(dir.path().join("tmp")).unwrap().count(), 0);
    // Wrong content is not gone on from.
    let other = BlobHash::of(b"other");
    let mut w = store.resume(&other).unwrap();
    w.write(b"nope").unwrap();
    assert!(w.finish(Some(&other)).is_err());
    assert_eq!(store.resume(&other).unwrap().size(), 0);
  }

  #[test]
  fn a_written_blob_must_match_its_expected_hash() {
    let dir = tempfile::tempdir().unwrap();
    let store = BlobStore::open(dir.path()).unwrap();
    let expected = BlobHash::of(b"right");
    let mut w = store.writer().unwrap();
    w.write(b"wrong").unwrap();
    assert!(w.finish(Some(&expected)).is_err());
    assert!(!store.contains(&expected));
    let mut w = store.writer().unwrap();
    w.write(b"ri").unwrap();
    w.write(b"ght").unwrap();
    assert_eq!(w.finish(Some(&expected)).unwrap(), (expected, 5));
    assert!(store.contains(&expected));
    assert_eq!(fs::read_dir(dir.path().join("tmp")).unwrap().count(), 0);
  }
}
