use super::source::Source;
use serde::{Deserialize, Serialize};
use windows::{
    Win32::{
        Foundation::ERROR_NOT_FOUND,
        Security::Credentials::{CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree, CredReadW, CredWriteW},
    },
    core::{PCWSTR, PWSTR},
};
use zeroize::Zeroize;

/// 仅用于缓存归属；序列化敏感材料使用后立即清零，指纹不返回页面。
pub(super) fn cache_identity(credentials: &Credentials) -> Result<String, String>
{
    use base64::Engine;
    use windows::Win32::Security::Cryptography::{BCRYPT_SHA256_ALG_HANDLE, BCryptHash};
    let mut input = serde_json::to_vec(credentials).map_err(|_| "天气配置指纹编码失败")?; // 各来源字段集合不相交，指纹不会跨来源相同。
    let mut digest = [0_u8; 32]; // 仅存储不可逆摘要，不保存原始凭据。
    let result = unsafe { BCryptHash(BCRYPT_SHA256_ALG_HANDLE, None, &input, &mut digest) };
    input.zeroize();
    result.ok().map_err(|_| "天气配置指纹计算失败")?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest))
}

/// 和风账户凭据；只在设置命令和 Windows 凭据存储之间传递。
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QweatherCredentials
{
    pub host: String,          // 用户账户的官方 API Host。
    pub developer_id: String,  // JWT iss。
    pub project_id: String,    // JWT sub。
    pub credential_id: String, // JWT kid。
    pub private_key: String,   // PKCS#8 Ed25519 私钥草稿。
}

impl Drop for QweatherCredentials
{
    fn drop(&mut self) { self.private_key.zeroize(); }
}

/// Foreca 账户凭据；该来源只使用一个 API key。
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForecaCredentials
{
    pub api_key: String, // 控制台签发的 Bearer 密钥。
}

impl Drop for ForecaCredentials
{
    fn drop(&mut self) { self.api_key.zeroize(); }
}

/// 按来源区分的凭据；两个变体字段集合不相交，因此可以按结构判定期望来源。
#[derive(Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Credentials
{
    Qweather(QweatherCredentials),
    Foreca(ForecaCredentials),
}

impl Credentials
{
    /// 按凭据结构判定所属来源，供验证标签与记录名共用。
    pub fn source(&self) -> Source
    {
        match self
        {
            Credentials::Qweather(_) => Source::Qweather,
            Credentials::Foreca(_) => Source::Foreca,
        }
    }
}

/// 每个来源一条独立记录；和风沿用既有记录名称，已保存凭据不因新增来源失效。
/// 读写两侧都必须经由本函数取名，否则改名会出现“写 A 读 B”，表现为已配置却显示未配置。
pub fn record_name(namespace: &str, source: Source) -> String
{
    match source
    {
        Source::Qweather => namespace.to_string(), // 和风沿用应用级记录名，迁移前的凭据继续有效。
        source => format!("{}/{}", namespace, source.key()),
    }
}

/// 每个来源一条独立记录；和风沿用既有记录名称，已保存凭据不因新增来源失效。
pub fn target(source: Source) -> String { record_name("DesktopTools/Weather", source) }

/// 查询指定记录是否已保存，清零系统返回的敏感缓冲区后立即释放。
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

/// 从 Windows 凭据存储读取指定记录，并按来源解析其结构。
pub fn load(source: Source, target: &str) -> Result<Option<Credentials>, String>
{
    let name: Vec<u16> = target.encode_utf16().chain(Some(0)).collect();
    let mut credential = std::ptr::null_mut();
    unsafe {
        match CredReadW(PCWSTR(name.as_ptr()), CRED_TYPE_GENERIC, None, &mut credential)
        {
            Ok(()) =>
            {
                if (*credential).CredentialBlob.is_null() || (*credential).CredentialBlobSize == 0
                {
                    CredFree(credential.cast());
                    return Err("天气凭据内容为空".into());
                }
                let bytes = std::slice::from_raw_parts((*credential).CredentialBlob, (*credential).CredentialBlobSize as usize);
                let result = decode(source, bytes);
                if !(*credential).CredentialBlob.is_null()
                {
                    std::slice::from_raw_parts_mut((*credential).CredentialBlob, (*credential).CredentialBlobSize as usize).zeroize();
                }
                CredFree(credential.cast());
                result.map(Some)
            }
            Err(error) if error.code() == ERROR_NOT_FOUND.to_hresult() => Ok(None),
            Err(_) => Err("Windows 凭据存储不可用，请稍后重试".into()),
        }
    }
}

/// 按记录所属来源解析，不接受跨来源的字段组合。
fn decode(source: Source, bytes: &[u8]) -> Result<Credentials, String>
{
    match source
    {
        Source::Qweather => serde_json::from_slice::<QweatherCredentials>(bytes).map(Credentials::Qweather),
        Source::Foreca => serde_json::from_slice::<ForecaCredentials>(bytes).map(Credentials::Foreca),
    }
    .map_err(|_| "天气凭据格式无效".to_string())
}

/// 单条记录原子替换；写入失败不删除旧凭据。
pub fn save(target: &str, credentials: &Credentials) -> Result<(), String>
{
    let mut blob = match credentials
    {
        Credentials::Qweather(value) => serde_json::to_vec(value),
        Credentials::Foreca(value) => serde_json::to_vec(value),
    }
    .map_err(|_| "凭据编码失败")?; // 记录不写入来源字段，来源由记录名称确定。
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

/// 明确清除指定记录，不影响其他来源与应用配置。
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
