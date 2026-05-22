use super::SecureStorage;

#[test]
fn test_encrypt_decrypt_returns_same_value() {
    let key = String::from("key");
    let inputs = [
        "freckles grain uncaring strict stumbling reappear basil uproar",
        "ideology shifting overview cognition uniformed armory mummify editor",
        "",
        "{",
        "\'",
        "\"",
        "{\"test\"}",
        "defender french skating sweat neurotic extruding cadet mute headcount unaligned prognosis heroics geography deafening customer juicy scuttle blissful scrambler spleen embark engine shield banter botanist singing plutonium grafted carton playable approve astonish",
        "{\"id_token\":{\"id_token\":\"This is an ID token.\",\"refresh_token\":\"This is a refresh token.\",\"expiration_time\":\"2025-10-22T15:26:51.091844800-04:00\"},\"refresh_token\":\"\",\"local_id\":\"test_user_uid\",\"email\":\"test_user@warp.dev\",\"display_name\":\"abcdef\",\"photo_url\":\"some-photo-url=\",\"is_onboarded\":true,\"needs_sso_link\":false,\"anonymous_user_type\":null,\"expires_at\":null,\"linked_at\":null,\"is_guaranteed_expired\":false,\"is_on_work_domain\":false}",
    ].map(String::from);

    fn encrypt_then_decrypt(key: &str, input: String) -> String {
        let encrypted = SecureStorage::encrypt(key, input).unwrap();
        SecureStorage::decrypt(encrypted).unwrap()
    }

    for input in inputs {
        assert_eq!(
            encrypt_then_decrypt(&key, input.to_owned()),
            input,
            "Encrypting and decrypting {input:?}"
        );
    }
}
