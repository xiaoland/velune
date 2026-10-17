use serde::{Deserialize, Serialize};

/// Transport errors deliberately contain no request body, credentials, paths, or upstream errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "camelCase")]
pub enum RemoteError {
    #[error("远控配置无效")]
    Configuration,
    #[error("无法读取或安全保存设备身份")]
    IdentityStorage,
    #[error("无法读取或保存可信设备")]
    TrustStorage,
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
