//! Windows Credential Manager 的通用凭据读写，供天气与 loopback 通道共用。

use windows::{
    Win32::{
        Foundation::ERROR_NOT_FOUND,
        Security::Credentials::{CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree, CredReadW, CredWriteW},
    },
    core::{PCWSTR, PWSTR},
};
use zeroize::Zeroize;

/// 单条通用凭据允许的最大字节数（Windows Credential Manager 上限）。
pub(crate) const MAX_SECRET_BYTES: usize = 2560;

/// 读取指定目标的通用凭据原始内容；记录不存在时返回 `None`。
pub(crate) fn read_secret(target: &str) -> Result<Option<Vec<u8>>, String>
{
    let name: Vec<u16> = target.encode_utf16().chain(Some(0)).collect(); // 原生目标名称。
    let mut credential = std::ptr::null_mut(); // CredReadW 分配的记录。
    unsafe {
        match CredReadW(PCWSTR(name.as_ptr()), CRED_TYPE_GENERIC, None, &mut credential)
        {
            Ok(()) =>
            {
                let record = &*credential;
                let bytes = if record.CredentialBlob.is_null() || record.CredentialBlobSize == 0
                {
                    Vec::new()
                }
                else
                {
                    std::slice::from_raw_parts(record.CredentialBlob, record.CredentialBlobSize as usize).to_vec()
                };
                if !record.CredentialBlob.is_null()
                {
                    std::slice::from_raw_parts_mut(record.CredentialBlob, record.CredentialBlobSize as usize).zeroize(); // 系统缓冲区用后清零。
                }
                CredFree(credential.cast());
                Ok(Some(bytes))
            }
            Err(error) if error.code() == ERROR_NOT_FOUND.to_hresult() => Ok(None),
            Err(_) => Err("Windows 凭据存储不可用".to_string()),
        }
    }
}

/// 写入指定目标的通用凭据；调用方负责在返回后清零自己的缓冲区。
pub(crate) fn write_secret(target: &str, blob: &mut [u8]) -> Result<(), String>
{
    if blob.is_empty() || blob.len() > MAX_SECRET_BYTES
    {
        return Err("凭据内容长度无效".to_string());
    }
    let mut name: Vec<u16> = target.encode_utf16().chain(Some(0)).collect(); // 原生目标名称。
    let record = CREDENTIALW { Type: CRED_TYPE_GENERIC, TargetName: PWSTR(name.as_mut_ptr()), CredentialBlobSize: blob.len() as u32, CredentialBlob: blob.as_mut_ptr(), Persist: CRED_PERSIST_LOCAL_MACHINE, ..Default::default() }; // 本机持久通用凭据。
    unsafe { CredWriteW(&record, 0) }.map_err(|_| "Windows 凭据保存失败".to_string())
}

/// 删除指定目标的通用凭据；记录不存在视为成功。
pub(crate) fn clear_secret(target: &str) -> Result<(), String>
{
    let name: Vec<u16> = target.encode_utf16().chain(Some(0)).collect(); // 原生目标名称。
    match unsafe { CredDeleteW(PCWSTR(name.as_ptr()), CRED_TYPE_GENERIC, None) }
    {
        Ok(()) => Ok(()),
        Err(error) if error.code() == ERROR_NOT_FOUND.to_hresult() => Ok(()),
        Err(_) => Err("Windows 凭据清除失败".to_string()),
    }
}
