use super::*;

#[test]
fn raised_fd_limit_is_the_hard_limit_capped() {
    assert_eq!(raised_fd_limit(256, 4096), 4096);
    assert_eq!(raised_fd_limit(256, 1 << 20), FD_LIMIT_CAP);
    assert_eq!(raised_fd_limit(256, libc::RLIM_INFINITY), FD_LIMIT_CAP);
}

#[test]
fn raised_fd_limit_never_lowers_the_soft_limit() {
    assert_eq!(raised_fd_limit(1 << 20, libc::RLIM_INFINITY), 1 << 20);
}
