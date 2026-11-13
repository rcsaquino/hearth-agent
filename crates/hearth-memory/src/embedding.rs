//! Local text embeddings for hybrid recall.
//!
//! The default model is `bekko-embedding-v1-a8m`: a 4-layer multilingual encoder
//! exported as ONNX. The model and tokenizer live in a local directory; there is
//! no network call on the recall path. A missing or broken model is a fallback,
//! not a daemon failure.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use ort::session::Session;
use ort::value::Tensor;
use tokenizers::Tokenizer;

use crate::errors::{ErrorCode, ExpectedError, Result};

/// The embedding model this build expects.
pub const MODEL_NAME: &str = "bekko-embedding-v1-a8m";
/// The model's output width.
pub const MODEL_DIMS: usize = 384;
/// Longest text sent to the encoder. The model supports more, but memories are
/// short and this bounds CPU time.
const MAX_TOKENS: usize = 512;

/// A text embedder usable from recall and indexing.
pub trait EmbeddingProvider: Send + Sync {
    fn model(&self) -> &str;
    fn dims(&self) -> usize;
    fn embed(&self, text: &str) -> Result<Vec<f32>>;
}

/// A local ONNX encoder.
pub struct BekkoEmbedding {
    directory: PathBuf,
    tokenizer: Tokenizer,
    session: Mutex<Session>,
}

impl BekkoEmbedding {
    /// Open a model directory holding `model.onnx` and `tokenizer.json`.
    ///
    /// `Ok(None)` means the files are absent, which selects lexical fallback.
    pub fn open(directory: impl Into<PathBuf>) -> Result<Option<Self>> {
        let directory = directory.into();
        let model = directory.join("model.onnx");
        let tokenizer = directory.join("tokenizer.json");
        if !model.is_file() || !tokenizer.is_file() {
            return Ok(None);
        }
        let tokenizer = Tokenizer::from_file(&tokenizer)
            .map_err(|error| embedding_error(format!("tokenizer could not be loaded: {error}")))?;
        let environment = ort::init()
            .build()
            .map_err(|error| embedding_error(format!("ONNX environment failed: {error}")))?;
        let session = Session::builder(&environment)
            .and_then(|mut builder| builder.commit_from_file(&model))
            .map_err(|error| {
                embedding_error(format!("embedding model could not be loaded: {error}"))
            })?;
        Ok(Some(Self {
            directory,
            tokenizer,
            session: Mutex::new(session),
        }))
    }

    /// The directory backing this model.
    pub fn directory(&self) -> &Path {
        &self.directory
    }
}

impl EmbeddingProvider for BekkoEmbedding {
    fn model(&self) -> &str {
        MODEL_NAME
    }

    fn dims(&self) -> usize {
        MODEL_DIMS
    }

    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let encoding = self
            .tokenizer
            .encode(text, true)
            .map_err(|error| embedding_error(format!("text could not be tokenized: {error}")))?;
        let mut ids: Vec<i64> = encoding
            .get_ids()
            .iter()
            .map(|id| i64::from(*id))
            .take(MAX_TOKENS)
            .collect();
        if ids.is_empty() {
            ids.push(0);
        }
        let len = ids.len();
        let attention: Vec<i64> = vec![1; len];
        let input_ids = Tensor::from_array(([1usize, len], ids))
            .map_err(|error| embedding_error(format!("input tensor failed: {error}")))?;
        let attention_mask = Tensor::from_array(([1usize, len], attention))
            .map_err(|error| embedding_error(format!("attention tensor failed: {error}")))?;
        let mut session = self
            .session
            .lock()
            .map_err(|_| embedding_error("embedding session lock poisoned".to_owned()))?;
        let outputs = session
            .run(ort::inputs![
                "input_ids" => input_ids,
                "attention_mask" => attention_mask
            ])
            .map_err(|error| embedding_error(format!("embedding inference failed: {error}")))?;
        let (shape, data) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|error| embedding_error(format!("embedding output failed: {error}")))?;
        let dims: Vec<usize> = shape.iter().map(|dimension| *dimension as usize).collect();
        let hidden = dims.last().copied().unwrap_or(MODEL_DIMS);
        let sequence = dims.get(1).copied().unwrap_or(1);
        if hidden != MODEL_DIMS || data.len() < sequence * hidden {
            return Err(embedding_error(
                "embedding output did not match the expected 384 dimensions".to_owned(),
            ));
        }
        let mut pooled = vec![0.0f32; hidden];
        for token in 0..sequence {
            for (index, value) in pooled.iter_mut().enumerate() {
                *value += data[token * hidden + index];
            }
        }
        let count = sequence as f32;
        for value in &mut pooled {
            *value /= count;
        }
        normalize(&mut pooled);
        Ok(pooled)
    }
}

fn normalize(vector: &mut [f32]) {
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in vector {
            *value /= norm;
        }
    }
}

fn embedding_error(message: String) -> ExpectedError {
    ExpectedError::new(ErrorCode::StoreUnavailable, message)
}

/// The pinned model revision and files used by the first-use downloader.
const MODEL_REVISION: &str = "c721113d59a1d91b447450324f51c4b3332c924a";
const MODEL_SHA: &str = "96d8cc6199e96357b21b2fb12f6d7ffd2d4abc7b182fe94b5468fbd6dc819af7";
const TOKENIZER_SHA: &str = "8bd47075711f75a143d1b78e01a41cc65c1c591b00d3cfeffc23db07adce1392";

/// Whether the two model files are present.
pub fn model_files_present(directory: &Path) -> bool {
    directory.join("model.onnx").is_file() && directory.join("tokenizer.json").is_file()
}

/// Download the pinned model into `directory`, replacing invalid files.
///
/// This runs only from the daemon's first-use path; recall never downloads.
pub fn download_model(directory: &Path) -> Result<()> {
    std::fs::create_dir_all(directory)?;
    let base = format!(
        "https://huggingface.co/hotchpotch/bekko-embedding-v1-a8m/resolve/{MODEL_REVISION}"
    );
    download_file(
        &format!("{base}/tokenizer.json"),
        &directory.join("tokenizer.json"),
        TOKENIZER_SHA,
    )?;
    download_file(
        &format!("{base}/onnx/model.onnx"),
        &directory.join("model.onnx"),
        MODEL_SHA,
    )?;
    Ok(())
}

fn download_file(url: &str, path: &Path, expected: &str) -> Result<()> {
    use sha2::{Digest, Sha256};
    use std::io::{Read, Write};

    if path.is_file()
        && let Ok(bytes) = std::fs::read(path)
    {
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        if format!("{:x}", hasher.finalize()) == expected {
            return Ok(());
        }
    }
    let temporary = path.with_extension("tmp");
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|error| embedding_error(format!("download client failed: {error}")))?;
    let mut response = client
        .get(url)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|error| embedding_error(format!("model download failed: {error}")))?;
    let mut file = std::fs::File::create(&temporary)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = response
            .read(&mut buffer)
            .map_err(|error| embedding_error(format!("model download failed: {error}")))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        file.write_all(&buffer[..read])?;
    }
    file.flush()?;
    drop(file);
    let actual = format!("{:x}", hasher.finalize());
    if actual != expected {
        let _ = std::fs::remove_file(&temporary);
        return Err(embedding_error(format!(
            "model checksum mismatch for {}",
            path.display()
        )));
    }
    std::fs::rename(&temporary, path)?;
    Ok(())
}
