use serde::Serialize;

/// 前后端共享的应用状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ApplicationStatus {
    /// 正式应用已就绪。
    Ready,
    /// 正式应用处于活动状态。
    Active,
}

/// 前后端共享的应用状态快照。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationSnapshot {
    /// 当前应用运行状态。
    pub status: ApplicationStatus,
    /// 当前快照修订号。
    pub revision: u64,
}

/// Tauri command 返回的结构化错误。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationError {
    /// 可稳定检索的错误码。
    pub code: String,
    /// 用户可见的中文错误说明。
    pub message: String,
}

/// 创建应用 command 使用的稳定错误。
impl ApplicationError {
    /// 创建非法状态参数错误。
    pub fn invalid_status() -> Self {
        Self {
            code: "invalid-application-status".to_string(),
            message: "应用状态参数无效".to_string(),
        }
    }

    /// 创建应用状态锁错误。
    pub fn state_unavailable() -> Self {
        Self {
            code: "application-state-unavailable".to_string(),
            message: "应用状态暂时不可用".to_string(),
        }
    }

    /// 创建应用事件发布错误。
    pub fn event_publish_failed(error: &impl std::fmt::Display) -> Self {
        Self {
            code: "application-event-publish-failed".to_string(),
            message: format!("应用状态事件发布失败: {error}"),
        }
    }
}

/// 应用快照变化事件名称。
pub const APPLICATION_SNAPSHOT_CHANGED_EVENT: &str = "application-snapshot-changed";

/// 校验前端提交的应用状态参数。
pub fn parse_application_status(value: &str) -> Result<ApplicationStatus, ApplicationError> {
    match value {
        "ready" => Ok(ApplicationStatus::Ready),
        "active" => Ok(ApplicationStatus::Active),
        _ => Err(ApplicationError::invalid_status()),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ApplicationError, ApplicationSnapshot, ApplicationStatus, parse_application_status,
    };

    /// 验证 command 接受公开状态值。
    #[test]
    fn command_accepts_a_known_application_status() {
        // command 输入解析结果。
        let status = parse_application_status("active");

        assert_eq!(status, Ok(ApplicationStatus::Active));
    }

    /// 验证 command 拒绝未知状态并返回稳定错误。
    #[test]
    fn command_rejects_an_unknown_application_status() {
        // command 输入错误结果。
        let error = parse_application_status("starting");

        assert_eq!(
            error,
            Err(ApplicationError {
                code: "invalid-application-status".to_string(),
                message: "应用状态参数无效".to_string(),
            }),
        );
    }

    /// 验证 event 载荷与前端契约字段一致。
    #[test]
    fn event_payload_uses_the_public_contract_shape() {
        // 等待序列化的应用快照。
        let snapshot = ApplicationSnapshot {
            status: ApplicationStatus::Active,
            revision: 7,
        };
        // event JSON 载荷。
        let payload = serde_json::to_value(snapshot).expect("应用快照应可序列化");

        assert_eq!(
            payload,
            serde_json::json!({ "status": "active", "revision": 7 })
        );
    }

    /// 验证 command 错误使用公开序列化契约。
    #[test]
    fn command_error_uses_the_public_contract_shape() {
        // command JSON 错误载荷。
        let payload =
            serde_json::to_value(ApplicationError::invalid_status()).expect("应用错误应可序列化");

        assert_eq!(
            payload,
            serde_json::json!({ "code": "invalid-application-status", "message": "应用状态参数无效" })
        );
    }
}
