use std::collections::BTreeMap;
use std::path::Path;

use runx_contracts::Receipt;

use crate::receipts::paths::{
    ReceiptPathInputs, ResolvedReceiptPath, RuntimeReceiptConfig, resolve_receipt_path,
};
use crate::receipts::store::{LocalReceiptStore, ReceiptStoreError};
use crate::receipts::{RuntimeReceiptSignatureConfig, RuntimeReceiptSigningError};
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

    pub(crate) fn signature_config(&self) -> &RuntimeReceiptSignatureConfig {
        &self.signature_config
    }

    #[cfg(test)]
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

    #[cfg(feature = "mcp")]
    pub(crate) fn write_local_receipt_dir(
        &self,
        receipt: &Receipt,
        receipt_dir: &Path,
    ) -> Result<(), ReceiptStoreError> {
        LocalReceiptStore::new(receipt_dir)
            .write_receipt_with_policy(receipt, self.signature_config.signature_policy())
    }
}
