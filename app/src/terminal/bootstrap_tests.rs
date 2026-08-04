use super::*;

struct TestAssetProvider;

impl AssetProvider for TestAssetProvider {
    fn get(&self, path: &str) -> anyhow::Result<Cow<'_, [u8]>> {
        let content = match path {
            "bundled/bootstrap/bash.sh" => "#include hello_world",
            "bundled/bootstrap/fish.sh" => "# this is a comment\nthis_is_a_command",
            "bundled/bootstrap/zsh.sh" => {
                "asdf\n#include whitespace\n    prepended whitespace\n\n\n"
            }
            "bundled/bootstrap/pwsh.ps1" => {
                r#"# This is a comment
                Write-Output 'Testing some output'
                function test1 {
                    [Diagnostics.CodeAnalysis.SuppressMessageAttribute('PSAvoidUsingInvokeExpression', '', Justification = 'We actually need it')]
                    param([string]$command)
                    Invoke-Expression $command
                }"#
            }
            "hello_world" => "hello world!",
            "whitespace" => "no whitespace\n\n\n yes whitespace!",
            _ => anyhow::bail!("path not found in assets"),
        };
        Ok(Cow::Borrowed(content.as_bytes()))
    }
}

/// A second, distinct `AssetProvider` type that returns different content for
/// the same paths as `TestAssetProvider`. Used to prove `BOOTSTRAP_CACHE` is
/// keyed on the concrete provider, not just the `ShellType`.
struct OtherAssetProvider;

impl AssetProvider for OtherAssetProvider {
    fn get(&self, path: &str) -> anyhow::Result<Cow<'_, [u8]>> {
        let content = match path {
            "bundled/bootstrap/bash.sh" => "#include hello_world",
            "hello_world" => "goodbye world!",
            _ => anyhow::bail!("path not found in assets"),
        };
        Ok(Cow::Borrowed(content.as_bytes()))
    }
}

/// A single `AssetProvider` type whose contents vary per instance. Used to
/// prove that a provider declaring a per-instance [`AssetCacheKey`] is not
/// served another instance's cached script.
struct VaryingAssetProvider {
    id: u64,
    greeting: &'static str,
}

impl AssetProvider for VaryingAssetProvider {
    fn get(&self, path: &str) -> anyhow::Result<Cow<'_, [u8]>> {
        let content = match path {
            "bundled/bootstrap/bash.sh" => "#include hello_world",
            "hello_world" => self.greeting,
            _ => anyhow::bail!("path not found in assets"),
        };
        Ok(Cow::Borrowed(content.as_bytes()))
    }

    fn cache_key(&self) -> AssetCacheKey {
        AssetCacheKey::for_instance::<Self>(self.id)
    }
}

#[test]
fn test_include_directive() {
    assert_eq!(
        decode_script(&script_for_shell(ShellType::Bash, &TestAssetProvider)),
        "hello world!\n"
    );
}

#[test]
fn test_trims_comments() {
    assert_eq!(
        decode_script(&script_for_shell(ShellType::Fish, &TestAssetProvider)),
        "this_is_a_command\n"
    );
}

#[test]
fn test_trims_whitespace() {
    assert_eq!(
        decode_script(&script_for_shell(ShellType::Zsh, &TestAssetProvider)),
        "asdf\nno whitespace\n yes whitespace!\n prepended whitespace\n"
    );
}

// Regression test for #13974: `BOOTSTRAP_CACHE` must be keyed on the concrete
// `AssetProvider` (its `TypeId`), not just the `ShellType`. Before the fix the
// first provider to populate a given `ShellType` entry won that entry for the
// rest of the process, so a second provider for the same shell incorrectly
// received the first provider's cached script. This test is order-independent:
// it asserts within a single test that two distinct providers for the same
// `ShellType` each get their own script (before the fix the second assertion
// fails, seeing the first provider's `"hello world!"` instead of
// `"goodbye world!"`).
#[test]
fn test_cache_is_keyed_on_asset_provider() {
    assert_eq!(
        decode_script(&script_for_shell(ShellType::Bash, &TestAssetProvider)),
        "hello world!\n"
    );
    assert_eq!(
        decode_script(&script_for_shell(ShellType::Bash, &OtherAssetProvider)),
        "goodbye world!\n"
    );
}

// Two providers of the same concrete type with different backing data must not
// share a cache entry. Keying on `TypeId` alone collapses these two instances
// onto one entry, so the second assertion sees the first instance's script.
#[test]
fn test_cache_distinguishes_instances_of_one_provider_type() {
    let first = VaryingAssetProvider {
        id: 1,
        greeting: "first instance!",
    };
    let second = VaryingAssetProvider {
        id: 2,
        greeting: "second instance!",
    };

    assert_eq!(
        decode_script(&script_for_shell(ShellType::Bash, &first)),
        "first instance!\n"
    );
    assert_eq!(
        decode_script(&script_for_shell(ShellType::Bash, &second)),
        "second instance!\n"
    );
}

// A stateless provider's key must be stable across instances, so the memoized
// script is actually reused rather than recomputed per instance.
#[test]
fn test_default_cache_key_is_stable_across_instances() {
    assert_eq!(TestAssetProvider.cache_key(), TestAssetProvider.cache_key());
    assert_eq!(
        TestAssetProvider.cache_key(),
        AssetCacheKey::for_type::<TestAssetProvider>()
    );
    assert_ne!(
        TestAssetProvider.cache_key(),
        OtherAssetProvider.cache_key()
    );
}

#[test]
fn test_trims_powershell_specifics() {
    assert_eq!(
        decode_script(&script_for_shell(ShellType::PowerShell, &TestAssetProvider)),
        " Write-Output 'Testing some output'\n function test1 {\n param([string]$command)\n Invoke-Expression $command\n }\n"
    );
}

fn decode_script(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).expect("should not fail to decode")
}
