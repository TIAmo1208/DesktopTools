use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use windows::{
    Win32::{
        Foundation::ERROR_NOT_FOUND,
        Security::Credentials::{CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree, CredReadW, CredWriteW},
    },
    core::{PCWSTR, PWSTR},
};
use zeroize::Zeroize;

/// 只在设置命令和 Windows 凭据存储之间传递，不进入快照或日志。
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Credentials
{
    pub host: String,          // 用户账户的官方 API Host。
    pub developer_id: String,  // JWT iss。
    pub project_id: String,    // JWT sub。
    pub credential_id: String, // JWT kid。
    pub private_key: String,   // PKCS#8 Ed25519 私钥草稿。
}

impl Drop for Credentials
{
    fn drop(&mut self) { self.private_key.zeroize(); }
}

/// 按配置目录隔离凭据，目标名不包含账户参数。
pub fn target(directory: &std::path::Path) -> String
{
    let identity = directory.to_string_lossy().replace('/', "\\").to_lowercase(); // Windows 配置归属。
    format!("DesktopTools/Weather/{:x}", Sha256::digest(identity.as_bytes()))
}

/// 查询是否已保存，清零系统返回的敏感缓冲区后立即释放。
pub fn configured(target: &str) -> Result<bool, String>
{
    let name: Vec<u16> = target.encode_utf16().chain(Some(0)).collect(); // 原生目标名称。
    let mut credential = std::ptr::null_mut(); // CredReadW 分配的记录。
    unsafe {
        match CredReadW(PCWSTR(name.as_ptr()), CRED_TYPE_GENERIC, None, &mut credential)
        {
            Ok(()) =>
            {
                if !(*credential).CredentialBlob.is_null()
                {
                    std::slice::from_raw_parts_mut((*credential).CredentialBlob, (*credential).CredentialBlobSize as usize).zeroize();
                }
                CredFree(credential.cast());
                Ok(true)
            }
            Err(error) if error.code() == ERROR_NOT_FOUND.to_hresult() => Ok(false),
            Err(_) => Err("Windows 凭据存储不可用，请稍后重试".into()),
        }
    }
}

/// 单条记录原子替换；写入失败不删除旧凭据。
pub fn save(target: &str, credentials: &Credentials) -> Result<(), String>
{
    let mut blob = serde_json::to_vec(credentials).map_err(|_| "凭据编码失败")?; // 仅内存序列化。
    if blob.len() > 2560
    {
        blob.zeroize();
        return Err("凭据内容过长".into());
    }
    let mut name: Vec<u16> = target.encode_utf16().chain(Some(0)).collect(); // 原生目标名称。
    let record = CREDENTIALW { Type: CRED_TYPE_GENERIC, TargetName: PWSTR(name.as_mut_ptr()), CredentialBlobSize: blob.len() as u32, CredentialBlob: blob.as_mut_ptr(), Persist: CRED_PERSIST_LOCAL_MACHINE, ..Default::default() }; // 本机持久通用凭据。
    let result = unsafe { CredWriteW(&record, 0) }.map_err(|_| "Windows 凭据保存失败，原配置未改变".to_string()); // 不暴露原生输入。
    blob.zeroize();
    result
}

/// 明确清除当前配置的记录，不影响其他应用配置。
pub fn clear(target: &str) -> Result<(), String>
{
    let name: Vec<u16> = target.encode_utf16().chain(Some(0)).collect(); // 原生目标名称。
    match unsafe { CredDeleteW(PCWSTR(name.as_ptr()), CRED_TYPE_GENERIC, None) }
    {
        Ok(()) => Ok(()),
        Err(error) if error.code() == ERROR_NOT_FOUND.to_hresult() => Ok(()),
        Err(_) => Err("Windows 凭据清除失败，原配置未改变".into()),
    }
}
