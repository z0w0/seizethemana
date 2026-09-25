//! Core ML document inference for pre-converted ONNX models.

use anyhow::Context as _;
use fastembed::ModelTrait as _;
use ort::execution_providers::ExecutionProvider as _;
use sha2::Digest as _;
use tokenizers::{PaddingParams, PaddingStrategy};

/// Largest Core ML document batch to evaluate before the full corpus.
const MAX_BATCH_SIZE: usize = 32;

/// A Core ML document encoder for the production full-precision model.
pub struct CoreMlEmbedding {
    documents: fastembed::TextEmbedding,
    batch_size: usize,
    model_hash: String,
}

impl CoreMlEmbedding {
    /// Load a registered ONNX model with shapes suited to its quantization mode.
    pub fn load(
        models_dir: &std::path::Path,
        model: fastembed::EmbeddingModel,
        max_length: usize,
        profile: bool,
    ) -> anyhow::Result<Self> {
        let info = fastembed::EmbeddingModel::get_model_info(&model)
            .context("model is not in fastembed's registry")?;
        let quantization = fastembed::TextEmbedding::get_quantization_mode(&model);
        let batch_size = if quantization == fastembed::QuantizationMode::Dynamic {
            1
        } else {
            MAX_BATCH_SIZE
        };
        let pooling = fastembed::TextEmbedding::get_default_pooling_method(&model)
            .context("model has no registered pooling method")?;
        let repo = hf_hub::api::sync::ApiBuilder::new()
            .with_cache_dir(models_dir.to_path_buf())
            .build()?
            .model(info.model_code.clone());
        let onnx = std::fs::read(repo.get(&info.model_file)?)?;
        let model_hash = sha2::Sha256::digest(&onnx);
        let tokenizer_files = fastembed::TokenizerFiles {
            tokenizer_file: std::fs::read(repo.get("tokenizer.json")?)?,
            config_file: std::fs::read(repo.get("config.json")?)?,
            special_tokens_map_file: std::fs::read(repo.get("special_tokens_map.json")?)?,
            tokenizer_config_file: std::fs::read(repo.get("tokenizer_config.json")?)?,
        };
        let provider = ort::execution_providers::CoreML::default()
            .with_model_format(ort::execution_providers::coreml::ModelFormat::MLProgram)
            .with_compute_units(ort::execution_providers::coreml::ComputeUnits::CPUAndGPU)
            .with_static_input_shapes(true)
            .with_profile_compute_plan(profile)
            .with_model_cache_dir(
                models_dir
                    .join("coreml")
                    .join(format!("{model_hash:x}"))
                    .join(format!("batch-{batch_size}-tokens-{max_length}"))
                    .display()
                    .to_string(),
            );
        anyhow::ensure!(
            provider.is_available()?,
            "Core ML execution provider is unavailable"
        );
        let options = fastembed::InitOptionsUserDefined::new()
            .with_max_length(max_length)
            .with_intra_threads(super::INTRA_THREADS)
            .with_dimension_override("batch_size", batch_size as i64)
            .with_dimension_override("sequence_length", max_length as i64)
            .with_execution_providers(vec![provider.build()]);
        let mut user_model = fastembed::UserDefinedEmbeddingModel::new(onnx, tokenizer_files)
            .with_pooling(pooling)
            .with_quantization(quantization);
        user_model.output_key = info.output_key.clone();
        let mut documents =
            fastembed::TextEmbedding::try_new_from_user_defined(user_model, options)
                .context("loading Core ML model")?;
        let padding = documents
            .tokenizer
            .get_padding()
            .cloned()
            .context("model tokenizer has no padding configuration")?;
        documents.tokenizer.with_padding(Some(PaddingParams {
            strategy: PaddingStrategy::Fixed(max_length),
            ..padding
        }));
        Ok(Self {
            documents,
            batch_size,
            model_hash: format!("{model_hash:x}"),
        })
    }

    /// SHA-256 identity of the ONNX model used for both encoders.
    pub fn model_hash(&self) -> &str {
        &self.model_hash
    }

    /// Input rows per Core ML invocation.
    pub fn batch_size(&self) -> usize {
        self.batch_size
    }
}

impl super::DocumentEmbedder for CoreMlEmbedding {
    fn embed_documents(&mut self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        let mut vectors = Vec::with_capacity(texts.len());
        for chunk in texts.chunks(self.batch_size) {
            let mut batch = chunk.to_vec();
            batch.resize_with(self.batch_size, String::new);
            let mut output = self.documents.embed(&batch, Some(self.batch_size))?;
            anyhow::ensure!(
                output.len() == self.batch_size,
                "Core ML returned an incomplete batch"
            );
            output.truncate(chunk.len());
            for mut vector in output {
                super::normalize_row(&mut vector);
                vectors.push(vector);
            }
        }
        Ok(vectors)
    }
}
