//! Read-only private-key parsing. Never invokes converters or writes decrypted keys.
use base64::{engine::general_purpose::STANDARD, Engine};
use russh::keys::PrivateKey;
use std::{io::Read, path::Path};
use zeroize::Zeroizing;

const KEY_ERROR: &str = "SSH: Connected\nHost identity: Verified\nAuthentication: Failed (cannot read/decrypt private key; check format and passphrase)";
pub fn load(path: &Path, password: Option<&str>) -> Result<PrivateKey, String> {
    let file = std::fs::File::open(path).map_err(|_| KEY_ERROR)?;
    let mut text = Zeroizing::new(String::new());
    file.take(1024 * 1024 + 1)
        .read_to_string(&mut text)
        .map_err(|_| KEY_ERROR)?;
    if text.len() > 1024 * 1024 {
        return Err("Private key exceeds the 1 MiB limit".into());
    }
    decode(&text, password)
}

fn decode(text: &str, password: Option<&str>) -> Result<PrivateKey, String> {
    // russh 0.64.1's legacy PEM decoder supports only AES-128/RSA. Preserve
    // AES-128/AES-256 CBC RSA and EC files using RustCrypto primitives. This is
    // the legacy PEM EVP_BytesToKey(MD5, salt=IV[0..8], count=1) file-format KDF,
    // not an SSH signature/hash or negotiated cipher setting.
    if text.lines().any(|line| line == "Proc-Type: 4,ENCRYPTED") {
        let header = text.lines().next().ok_or(KEY_ERROR)?;
        if header != "-----BEGIN RSA PRIVATE KEY-----" && header != "-----BEGIN EC PRIVATE KEY-----"
        {
            return Err(KEY_ERROR.into());
        }
        let dek = text
            .lines()
            .find_map(|line| line.strip_prefix("DEK-Info: "))
            .ok_or(KEY_ERROR)?;
        let (cipher, hex) = dek.split_once(',').ok_or(KEY_ERROR)?;
        if cipher != "AES-128-CBC" && cipher != "AES-256-CBC" {
            return Err("Unsupported legacy PEM encryption. Use an encrypted OpenSSH or PKCS#8 key; the existing file was not changed.".into());
        }
        let mut iv = [0u8; 16];
        if hex.len() != 32 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(KEY_ERROR.into());
        }
        for (i, byte) in iv.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).map_err(|_| KEY_ERROR)?;
        }
        let password = password.ok_or(KEY_ERROR)?;
        let mut key = Zeroizing::new(Vec::new());
        let mut previous = Zeroizing::new(Vec::new());
        let key_length = if cipher == "AES-128-CBC" { 16 } else { 32 };
        while key.len() < key_length {
            let mut hash = md5::Context::new();
            hash.consume(previous.as_slice());
            hash.consume(password.as_bytes());
            hash.consume(&iv[..8]);
            previous.clear();
            previous.extend_from_slice(&hash.finalize().0);
            key.extend_from_slice(&previous);
        }
        let encoded = Zeroizing::new(
            text.lines()
                .skip_while(|line| !line.is_empty())
                .skip(1)
                .take_while(|line| !line.starts_with("-----END "))
                .collect::<String>(),
        );
        let mut plaintext =
            Zeroizing::new(STANDARD.decode(encoded.as_bytes()).map_err(|_| KEY_ERROR)?);
        use aes::cipher::{block_padding::Pkcs7, BlockModeDecrypt, KeyIvInit};
        let der = if key_length == 16 {
            cbc::Decryptor::<aes::Aes128>::new_from_slices(&key, &iv)
                .map_err(|_| KEY_ERROR)?
                .decrypt_padded::<Pkcs7>(&mut plaintext)
                .map_err(|_| KEY_ERROR)?
        } else {
            cbc::Decryptor::<aes::Aes256>::new_from_slices(&key, &iv)
                .map_err(|_| KEY_ERROR)?
                .decrypt_padded::<Pkcs7>(&mut plaintext)
                .map_err(|_| KEY_ERROR)?
        };
        let body = Zeroizing::new(STANDARD.encode(der));
        let pem = Zeroizing::new(format!(
            "{header}\n{}\n{}\n",
            body.as_str(),
            header.replace("BEGIN", "END")
        ));
        return russh::keys::decode_secret_key(&pem, None).map_err(|_| KEY_ERROR.into());
    }
    russh::keys::decode_secret_key(text, password).map_err(|_| KEY_ERROR.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn malformed_and_unsupported_encryption_never_expose_file_or_secret() {
        let cipher = "-----BEGIN RSA PRIVATE KEY-----\nProc-Type: 4,ENCRYPTED\nDEK-Info: DES-EDE3-CBC,0000000000000000\n\nAAAA\n-----END RSA PRIVATE KEY-----";
        assert!(decode(cipher, Some("secret-marker"))
            .unwrap_err()
            .contains("Unsupported legacy PEM encryption"));
        for input in ["secret-marker", "-----BEGIN RSA PRIVATE KEY-----\nProc-Type: 4,ENCRYPTED\nDEK-Info: AES-256-CBC,invalid\n\nAAAA"] {
            let error = decode(input, Some("secret-marker")).unwrap_err();
            assert!(!error.contains("secret-marker"));
            assert!(error.contains("Authentication: Failed"));
        }
        let directory = tempfile::tempdir().unwrap();
        let key = directory.path().join("oversized.key");
        std::fs::write(&key, vec![b'x'; 1024 * 1024 + 1]).unwrap();
        assert!(load(&key, None).unwrap_err().contains("1 MiB"));
    }
}
