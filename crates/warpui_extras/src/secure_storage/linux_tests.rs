use super::Error;
use super::SecureStorage;

#[test]
fn test_encrypt_decrypt_returns_same_value() {
    let storage = SecureStorage::new("darmok");

    let input = String::from("darmok and jalad at tanagra");
    let encrypted = storage.fallback_encrypt(&input).unwrap();
    let output = storage.fallback_decrypt(&encrypted).unwrap();

    assert_eq!(input, output)
}

#[test]
fn test_encrypt_decrypt_works_across_storage_instances() {
    let storage_1 = SecureStorage::new("darmok");
    let storage_2 = SecureStorage::new("jalad");

    let input = String::from("shaka when the walls fell");
    let encrypted = storage_1.fallback_encrypt(&input).unwrap();
    let output = storage_2.fallback_decrypt(&encrypted).unwrap();

    assert_eq!(input, output)
}

#[test]
fn test_decrypt_fails_on_malformed_data() {
    let storage = SecureStorage::new("darmok");

    let bad_datas: [&[u8]; 4] = [&[], &[0; 1], &[0; 11], &[0; 12]];

    for bad_data in bad_datas {
        let result = storage.fallback_decrypt(bad_data);

        assert!(result.is_err());
        let error = result.unwrap_err();
        let Error::Unknown(err) = error else {
            panic!("Expected error variant to be Error::Unknown, but found {error:?}")
        };
        assert_eq!(
            format!("{err}"),
            "Attempting to decrypt too small value for fallback decryption"
        );
    }
}
