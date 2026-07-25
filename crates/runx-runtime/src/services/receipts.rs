use std::collections::BTreeMap;
use std::path::Path;

use runx_contracts::Receipt;

use crate::RuntimeError;
use crate::receipts::paths::{
    ReceiptPathInputs, ResolvedReceiptPath, RuntimeReceiptConfig, resolve_receipt_path,
};
use crate::receipts::store::{LocalReceiptStore, ReceiptStoreError};
use crate::receipts::{
    Ed25519ReceiptVerifier, RUNX_RECEIPT_VERIFY_ED25519_PUBLIC_KEY_BASE64_ENV,
    RUNX_RECEIPT_VERIFY_KID_ENV, RuntimeReceiptSignatureConfig, RuntimeReceiptSigningError,
};
use crate::services::WorkspaceEnv;

#[derive(Clone, Debug)]
pub(crate) struct ReceiptServices {
    signature_config: RuntimeReceiptSignatureConfig,
}

impl ReceiptServices {
    pub(crate) fn from_env(
        env: &BTreeMap<String, String>,
    ) -> Result<Self, RuntimeReceiptSigningError> {
        Ok(Self {
            signature_config: RuntimeReceiptSignatureConfig::from_env(env)?,
        })
    }

    pub(crate) fn from_env_or_local_development(
        env: &BTreeMap<String, String>,
    ) -> Result<Self, RuntimeReceiptSigningError> {
        match RuntimeReceiptSignatureConfig::from_env(env) {
            Ok(signature_config) => Ok(Self { signature_config }),
            Err(RuntimeReceiptSigningError::MissingSigningEnv) => Ok(Self {
                signature_config: RuntimeReceiptSignatureConfig::local_development(),
            }),
            Err(error) => Err(error),
        }
    }

    pub(crate) fn signature_config(&self) -> &RuntimeReceiptSignatureConfig {
        &self.signature_config
    }

    #[cfg(any(feature = "cli-tool", test))]
    pub(crate) fn from_signature_config(signature_config: RuntimeReceiptSignatureConfig) -> Self {
        Self { signature_config }
    }

    pub(crate) fn resolve_path(
        &self,
        workspace: &WorkspaceEnv,
        explicit_dir: Option<&Path>,
        runtime_config: Option<&RuntimeReceiptConfig>,
    ) -> ResolvedReceiptPath {
        let _ = self;
        resolve_receipt_path(ReceiptPathInputs {
            explicit_dir,
            runtime_config,
            env: workspace.env(),
            cwd: workspace.cwd(),
        })
    }

    pub(crate) fn write_local_receipt(
        &self,
        receipt: &Receipt,
        path: &ResolvedReceiptPath,
    ) -> Result<(), ReceiptStoreError> {
        LocalReceiptStore::new(&path.path)
            .write_receipt_with_policy(receipt, self.signature_config.signature_policy())
    }

    #[cfg(any(feature = "cli-tool", feature = "mcp"))]
    pub(crate) fn write_local_receipts<'a>(
        &self,
        receipts: impl IntoIterator<Item = &'a Receipt>,
        receipt_dir: &Path,
    ) -> Result<(), ReceiptStoreError> {
        LocalReceiptStore::new(receipt_dir)
            .write_receipts_with_policy(receipts, self.signature_config.signature_policy())
    }

    #[cfg(feature = "cli-tool")]
    pub(crate) fn list_local_receipts(
        &self,
        receipt_dir: &Path,
    ) -> Result<Vec<Receipt>, ReceiptStoreError> {
        LocalReceiptStore::new(receipt_dir)
            .list_with_policy(self.signature_config.signature_policy())
    }

    #[cfg(feature = "cli-tool")]
    pub(crate) fn read_local_receipt(
        &self,
        receipt_id: &str,
        receipt_dir: &Path,
    ) -> Result<Receipt, ReceiptStoreError> {
        LocalReceiptStore::new(receipt_dir)
            .read_exact_with_policy(receipt_id, self.signature_config.signature_policy())
    }

    #[cfg(feature = "mcp")]
    pub(crate) fn write_local_receipt_dir(
        &self,
        receipt: &Receipt,
        receipt_dir: &Path,
    ) -> Result<(), ReceiptStoreError> {
        self.write_local_receipts(std::iter::once(receipt), receipt_dir)
    }
}

pub(crate) fn production_receipt_verifier(
    env: &BTreeMap<String, String>,
) -> Result<Option<Ed25519ReceiptVerifier>, RuntimeError> {
    let kid = non_empty_env(env, RUNX_RECEIPT_VERIFY_KID_ENV);
    let public_key = non_empty_env(env, RUNX_RECEIPT_VERIFY_ED25519_PUBLIC_KEY_BASE64_ENV);
    match (kid, public_key) {
        (None, None) => Ok(None),
        (Some(kid), Some(public_key)) => {
            Ed25519ReceiptVerifier::from_public_key_base64(kid.to_owned(), public_key)
                .map(Some)
                .map_err(|_| receipt_read_error("receipt verification public key is malformed"))
        }
        _ => Err(receipt_read_error(
            "receipt verification key id and public key must be configured together",
        )),
    }
}

fn non_empty_env<'a>(env: &'a BTreeMap<String, String>, key: &str) -> Option<&'a str> {
    env.get(key)
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn receipt_read_error(message: impl Into<String>) -> RuntimeError {
    RuntimeError::SkillFailed {
        skill_name: "receipt.read".to_owned(),
        message: message.into(),
    }
}
