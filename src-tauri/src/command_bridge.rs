use std::sync::Arc;

use serde_json::Value;

/// 页面命令处理器：平台只负责转发页面命令，来源白名单、参数校验与序列化在应用命令边界。
pub(crate) type NativeCommandHandler = Arc<dyn Fn(&str, Value) -> Result<Value, Value> + Send + Sync + 'static>;
