use crate::InvalidContract;

// No Deserialize on validated types: external configuration must use constructors.
macro_rules! id {
    ($($name:ident),+ $(,)?) => {$ (
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);
        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, InvalidContract> {
                let value=value.into();
                if value.is_empty() || value.len()>128 || !value.bytes().all(|c| c.is_ascii_alphanumeric() || b"-_.:/".contains(&c)) {
                    return Err(InvalidContract(concat!(stringify!($name), ": expected 1..128 identifier characters")));
                }
                Ok(Self(value))
            }
            pub fn as_str(&self) -> &str { &self.0 }
        }
    )+};
}
id!(ProviderId, ModelId, CallId, AttemptId, ToolCallId, ToolName);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfigRevision(std::num::NonZeroU64);
impl ConfigRevision {
    pub fn new(value: u64) -> Result<Self, InvalidContract> {
        std::num::NonZeroU64::new(value)
            .map(Self)
            .ok_or(InvalidContract("configuration revision must be nonzero"))
    }
    pub fn get(self) -> u64 {
        self.0.get()
    }
}
