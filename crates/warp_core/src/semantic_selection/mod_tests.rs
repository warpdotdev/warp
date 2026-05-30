use super::*;

#[test]
fn test_basic_url_selection() {
    let sel = SemanticSelection::mock(true, "");
    assert_eq!(
        sel.smart_search("http://stackoverflow.com foo", 5.into()),
        Some(ByteOffset::range(0..24))
    );
    assert_eq!(
        sel.smart_search("click here:http://stackoverflow.com", 15.into()),
        Some(ByteOffset::range(11..35))
    );
    assert_eq!(
            sel.smart_search("word here https://andy:foo@stackoverflow.com/questions/28265036/how?foo=bar&baz=food#thing-here other/stuff", 35.into()),
            Some(ByteOffset::range(10..95))
        );
}

#[test]
fn test_other_url_selection() {
    let sel = SemanticSelection::mock(true, "");
    assert_eq!(
        sel.smart_search("ssh://git@github.com:acarl005/dotfiles.git", 0.into()),
        Some(ByteOffset::range(0..42))
    );
    assert_eq!(
        sel.smart_search("data hdfs://hadoopNS/data/users.csv there", 20.into()),
        Some(ByteOffset::range(5..35))
    );
    assert_eq!(
        sel.smart_search("send files here&ftp://thing.网站/file please", 30.into()),
        Some(ByteOffset::range(16..39))
    );
    assert_eq!(
        sel.smart_search("http://[2001:db8::1]:80", 10.into()),
        Some(ByteOffset::range(0..23))
    );
}

#[test]
fn test_email_selection() {
    let sel = SemanticSelection::mock(true, "");
    assert_eq!(
        sel.smart_search(
            "mail to: acarl005@g.ucla.edu andy+1.hello@warp.dev",
            15.into()
        ),
        Some(ByteOffset::range(9..28))
    );
    assert_eq!(
        sel.smart_search(
            "mail to: acarl005@g.ucla.edu andy+1.hello@warp.dev",
            33.into()
        ),
        Some(ByteOffset::range(29..50))
    );
}

#[test]
fn test_numerical_selection() {
    let sel = SemanticSelection::mock(true, "");
    assert_eq!(
        sel.smart_search("6.02e-23 -8.1e1 3.1415 8.1e+10-3.1415", 2.into()),
        Some(ByteOffset::range(0..8))
    );
    assert_eq!(
        sel.smart_search("6.02e-23 -8.1e1 3.1415 8.1e+10-3.1415", 11.into()),
        Some(ByteOffset::range(9..15))
    );
    assert_eq!(
        sel.smart_search("6.02e-23 -8.1e1 3.1415 8.1e+10-3.1415", 18.into()),
        Some(ByteOffset::range(16..22))
    );
    assert_eq!(
        sel.smart_search("6.02e-23 -8.1e1 3.1415 8.1e+10-3.1415", 27.into()),
        Some(ByteOffset::range(23..30))
    );
}

#[test]
fn test_filepath_selection() {
    let sel = SemanticSelection::mock(true, "");
    assert_eq!(
        sel.smart_search("~/.config/nvim/init.lua C:\\system\\file\\path", 2.into()),
        Some(ByteOffset::range(0..23))
    );
    assert_eq!(
        sel.smart_search("~/.config/nvim/init.lua C:\\system\\file\\path", 30.into()),
        Some(ByteOffset::range(24..43))
    );
    assert_eq!(
        sel.smart_search("scp ./foo.txt andy@ubuntu:/etc/foo.txt", 7.into()),
        Some(ByteOffset::range(4..13))
    );
    assert_eq!(
        sel.smart_search("scp ./foo.txt andy@ubuntu:/etc/foo.txt", 28.into()),
        Some(ByteOffset::range(26..38))
    );
}

#[test]
fn test_identifier_selection() {
    let sel = SemanticSelection::mock(true, "");
    assert_eq!(
        sel.smart_search("192.168.0.1:3000 api-service-pod foo-bar--baz", 6.into()),
        Some(ByteOffset::range(0..11))
    );
    assert_eq!(
        sel.smart_search("192.168.0.1:3000 api-service-pod foo-bar--baz", 30.into()),
        Some(ByteOffset::range(17..32))
    );
    assert_eq!(
        sel.smart_search("192.168.0.1:3000 api-service-pod foo-bar--baz", 33.into()),
        Some(ByteOffset::range(33..40))
    );
}
