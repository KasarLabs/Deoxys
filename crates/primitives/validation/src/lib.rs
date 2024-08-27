use dp_rayon_pool::RayonPool;
use starknet_api::core::ChainId;

// TODO: We should be able to pass any Context to the Validate trait - not only the one
// related to block validation context.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidationContext {
    /// Use the transaction hashes from the transaction receipts instead of computing them.
    pub trust_transaction_hashes: bool,
    pub chain_id: ChainId,
}

#[async_trait::async_trait]
pub trait Validate: Send + Sync + Sized + 'static {
    type Output: Send + 'static;
    type ValidationError: Send;

    fn validate(self, context: &ValidationContext) -> Result<Self::Output, Self::ValidationError>;

    async fn spawn_validate(
        self,
        pool: &RayonPool,
        context: ValidationContext,
    ) -> Result<Self::Output, Self::ValidationError> {
        pool.spawn_rayon_task(move || self.validate(&context)).await
    }
}