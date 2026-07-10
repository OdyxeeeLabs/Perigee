use soroban_sdk::contracttype;

/// Errors that can occur during cross-chain payload verification
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CrossChainError {
    /// Payload hash does not match
    InvalidPayloadHash,
    /// One or more signatures are invalid
    InvalidSignature,
    /// Not enough signatures to reach consensus
    InsufficientSignatures,
    /// Signature verification failed with unknown error
    SignatureVerificationFailed,
    /// Payload has expired
    PayloadExpired,
    /// Payload hash already verified (replay attack detected)
    ReplayAttack,
    /// Validator set is invalid or missing
    InvalidValidatorSet,
    /// Validator is not in the active set
    ValidatorNotInSet,
    /// Sender is not authorized to execute payload
    UnauthorizedSender,
    /// Recipient chain or address is invalid
    InvalidRecipient,
    /// Source chain is not recognized
    UnknownSourceChain,
    /// Destination chain is not accessible
    InaccessibleDestinationChain,
    /// Bridge between chains is disabled or inactive
    BridgeInactive,
    /// Payload data is malformed
    MalformedPayload,
    /// Encoding/decoding of payload failed
    EncodingError,
    /// Operation is not supported
    UnsupportedOperation,
    /// Gas limit is too low for execution
    InsufficientGas,
    /// Verification context is missing required data
    IncompleteVerificationContext,
    /// Nonce has already been used (replay protection)
    NonceAlreadyUsed,
    /// Timestamp is too far in the past or future
    InvalidTimestamp,
    /// Sequence number is out of order
    SequenceOutOfOrder,
    /// Cross-chain contract is in maintenance mode
    MaintenanceMode,
    /// Generic verification failure
    VerificationFailed,
    /// Too many payloads pending verification
    BacklogExceeded,
    /// Bridge fee validation failed
    FeeValidationFailed,
    /// Liquidity pool error
    LiquidityError,
    /// Storage operation failed
    StorageError,
    /// Unauthorized operation
    Unauthorized,
    /// Generic error
    Unknown,
}

impl CrossChainError {
    /// Convert error to a numeric code for external representation
    pub fn as_u32(&self) -> u32 {
        match self {
            Self::InvalidPayloadHash => 1,
            Self::InvalidSignature => 2,
            Self::InsufficientSignatures => 3,
            Self::SignatureVerificationFailed => 4,
            Self::PayloadExpired => 5,
            Self::ReplayAttack => 6,
            Self::InvalidValidatorSet => 7,
            Self::ValidatorNotInSet => 8,
            Self::UnauthorizedSender => 9,
            Self::InvalidRecipient => 10,
            Self::UnknownSourceChain => 11,
            Self::InaccessibleDestinationChain => 12,
            Self::BridgeInactive => 13,
            Self::MalformedPayload => 14,
            Self::EncodingError => 15,
            Self::UnsupportedOperation => 16,
            Self::InsufficientGas => 17,
            Self::IncompleteVerificationContext => 18,
            Self::NonceAlreadyUsed => 19,
            Self::InvalidTimestamp => 20,
            Self::SequenceOutOfOrder => 21,
            Self::MaintenanceMode => 22,
            Self::VerificationFailed => 23,
            Self::BacklogExceeded => 24,
            Self::FeeValidationFailed => 25,
            Self::LiquidityError => 26,
            Self::StorageError => 27,
            Self::Unauthorized => 28,
            Self::Unknown => 255,
        }
    }
}
