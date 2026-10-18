use serde::{Deserialize, Serialize};

/// Transport errors deliberately contain no request body, credentials, paths, or upstream errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "camelCase")]
pub enum RemoteError {
    #[error("远控配置无效")]
    Configuration,
    #[error("连接设置已被另一调用修改，请重新读取")]
    ConfigurationConflict,
    #[error("无法读取或安全保存设备身份")]
    IdentityStorage,
    #[error("无法读取或原子保存远控状态")]
    StateStorage,
    #[error("配对事务未授权、已失效或不可查询")]
    PairingUnavailable,
    #[error("请先处理此设备的未决请求")]
    PendingRequests,
    #[error("远程副作用必须使用一次操作作用域")]
    OperationRequired,
    #[error("本次操作作用域已经使用；请查询原请求")]
    OperationUsed,
    #[error("无法生成设备随机材料")]
    Randomness,
    #[error("配对邀请无效或已使用")]
    InvalidInvitation,
    #[error("配对邀请已过期")]
    ExpiredInvitation,
    #[error("设备未获授权或授权已撤销")]
    Unauthorized,
    #[error("远控连接不可用")]
    Unavailable,
    #[error("远控请求超时；已接纳的业务可能仍在执行")]
    Timeout,
    #[error("远控协议数据无效")]
    Protocol,
    #[error("远控协议版本不兼容")]
    IncompatibleVersion,
    #[error("执行端已重启，旧请求结果未知；不得自动重发")]
    ServerRestarted,
    #[error("远控请求标识无效")]
    InvalidRequestId,
    #[error("同一请求标识已用于不同的命令")]
    RequestIdConflict,
    #[error("远控容量已满，请稍后重试或复用已有请求会话")]
    Capacity,
    #[error("远控帧超过大小上限")]
    FrameTooLarge,
    #[error("远控入口已关闭")]
    Closed,
    #[error("执行端未装配业务处理器")]
    NoHandler,
}

pub type Result<T> = std::result::Result<T, RemoteError>;

impl RemoteError {
    /// Safe machine-readable cause, without request or infrastructure details.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Configuration => "configuration",
            Self::ConfigurationConflict => "configuration_conflict",
            Self::IdentityStorage => "identity_storage",
            Self::StateStorage => "state_storage",
            Self::PairingUnavailable => "pairing_unavailable",
            Self::PendingRequests => "pending_requests",
            Self::OperationRequired => "operation_required",
            Self::OperationUsed => "operation_used",
            Self::Randomness => "randomness",
            Self::InvalidInvitation => "invalid_invitation",
            Self::ExpiredInvitation => "expired_invitation",
            Self::Unauthorized => "unauthorized",
            Self::Unavailable => "unavailable",
            Self::Timeout => "timeout",
            Self::Protocol => "protocol",
            Self::IncompatibleVersion => "incompatible_version",
            Self::ServerRestarted => "server_restarted",
            Self::InvalidRequestId => "invalid_request_id",
            Self::RequestIdConflict => "request_id_conflict",
            Self::Capacity => "capacity",
            Self::FrameTooLarge => "frame_too_large",
            Self::Closed => "closed",
            Self::NoHandler => "no_handler",
        }
    }
}
