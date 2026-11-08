use anyhow::Result;
use base64::{Engine, engine::general_purpose::STANDARD};
use std::ptr::{null, null_mut};
use windows_sys::Win32::{
    Foundation::LocalFree,
    Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    },
};

pub(super) fn protect(secret: &str) -> Result<String> {
    Ok(STANDARD.encode(transform(secret.as_bytes(), true)?))
}

pub(super) fn unprotect(encoded: &str) -> Result<String> {
    let ciphertext = STANDARD.decode(encoded)?;
    Ok(String::from_utf8(transform(&ciphertext, false)?)?)
}

fn transform(bytes: &[u8], protect: bool) -> Result<Vec<u8>> {
    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes.len().try_into()?,
        pbData: bytes.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: null_mut(),
    };
    // DPAPI reads the input and allocates the result with LocalAlloc on success.
    unsafe {
        let flags = CRYPTPROTECT_UI_FORBIDDEN;
        let ok = if protect {
            CryptProtectData(&input, null(), null(), null(), null(), flags, &mut output)
        } else {
            CryptUnprotectData(
                &input,
                null_mut(),
                null(),
                null(),
                null(),
                flags,
                &mut output,
            )
        };
        anyhow::ensure!(
            ok != 0 && !output.pbData.is_null(),
            "Windows 凭据保护操作失败"
        );
        let bytes = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        LocalFree(output.pbData.cast());
        Ok(bytes)
    }
}
