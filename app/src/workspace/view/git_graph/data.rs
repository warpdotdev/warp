//! Git Graph 的数据层：提交数据类型、`git log` 输出解析（纯函数）与异步取数。
//!
//! 取数统一走 [`warp_util::git::run_git_command`]（shell 出 `git`），不依赖
//! `git2`/`gix`。解析逻辑与 IO 分离：`parse_*` 是纯函数，可独立单测；
//! `load_*` 只做"组装命令 + 调用解析"的薄封装。

#[cfg(not(target_family = "wasm"))]
use std::path::Path;

#[cfg(not(target_family = "wasm"))]
use anyhow::Result;

/// `git log --pretty=format` 中字段之间的分隔符（ASCII Unit Separator）。
/// 用控制字符而非可打印字符，避免 subject / ref 名里的普通字符破坏解析。
const UNIT_SEP: char = '\u{1f}';
/// 提交记录之间的分隔符（ASCII Record Separator）。
const RECORD_SEP: char = '\u{1e}';

/// 传给 `git log` 的格式串。字段顺序与 [`parse_commit_record`] 严格对应：
/// hash / parents / author name / author email / author time / decorate / subject。
/// `%x1f` `%x1e` 是 git 对上面两个分隔符字节的转义写法。
#[cfg(not(target_family = "wasm"))]
const LOG_FORMAT: &str = "--pretty=format:%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%D%x1f%s%x1e";

/// 一个提交节点，承载图谱一行所需的全部数据。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommitNode {
    /// 完整 commit hash。
    pub hash: String,
    /// 用于展示的短 hash（前 7 位）。
    pub short_hash: String,
    /// 父提交完整 hash 列表：0 个为根，2 个及以上为合并。
    pub parents: Vec<String>,
    pub author_name: String,
    pub author_email: String,
    /// 作者时间（Unix 秒）。
    pub author_time: i64,
    /// 提交信息首行。
    pub subject: String,
    /// 指向本提交的引用标签（分支 / 远程分支 / tag / HEAD）。
    pub refs: Vec<RefLabel>,
}

/// 引用标签的种类，决定渲染样式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RefKind {
    /// 当前检出的位置（`HEAD`）。
    Head,
    LocalBranch,
    RemoteBranch,
    Tag,
}

/// 指向某个提交的一个引用标签。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RefLabel {
    pub kind: RefKind,
    /// 展示名（已去除 `refs/heads/` 等前缀）。
    pub name: String,
}

/// 把 `git log`（[`LOG_FORMAT`] 格式）的整段输出解析为提交列表。
pub(crate) fn parse_commit_log(stdout: &str) -> Vec<CommitNode> {
    stdout
        .split(RECORD_SEP)
        .filter_map(|record| {
            // 记录之间 git 会以换行连接，去掉前后空白/换行后再解析。
            let record = record.trim_matches(|c: char| c == '\n' || c == '\r');
            if record.is_empty() {
                return None;
            }
            parse_commit_record(record)
        })
        .collect()
}

/// 解析单条提交记录。字段不足时返回 `None`（跳过该条而非 panic）。
fn parse_commit_record(record: &str) -> Option<CommitNode> {
    // 用 splitn(7) 保证最后的 subject 段即使含分隔符也完整保留。
    let mut fields = record.splitn(7, UNIT_SEP);
    let hash = fields.next()?.to_string();
    let parents_raw = fields.next()?;
    let author_name = fields.next()?.to_string();
    let author_email = fields.next()?.to_string();
    let author_time = fields.next()?.trim().parse::<i64>().ok()?;
    let decorate = fields.next()?;
    let subject = fields.next().unwrap_or("").to_string();

    let parents = parents_raw
        .split_whitespace()
        .map(str::to_string)
        .collect::<Vec<_>>();
    let short_hash = hash.chars().take(7).collect::<String>();
    let refs = parse_decorate(decorate);

    Some(CommitNode {
        hash,
        short_hash,
        parents,
        author_name,
        author_email,
        author_time,
        subject,
        refs,
    })
}

/// 解析 `%D`（`--decorate=full` 模式）的 decorate 串为引用标签列表。
///
/// 输入形如 `HEAD -> refs/heads/main, refs/remotes/origin/main, refs/tags/v1`。
/// 用 full 模式是为了可靠区分本地分支 / 远程分支 / tag（短模式无法区分
/// 本地的 `feature/x` 与远程的 `origin/x`）。
pub(crate) fn parse_decorate(decorate: &str) -> Vec<RefLabel> {
    let decorate = decorate.trim();
    if decorate.is_empty() {
        return Vec::new();
    }
    decorate
        .split(',')
        .filter_map(|raw| {
            let token = raw.trim();
            if token.is_empty() {
                return None;
            }
            // "HEAD -> refs/heads/main"：HEAD 当前所在分支。
            if let Some(branch) = token.strip_prefix("HEAD -> ") {
                let name = branch.strip_prefix("refs/heads/").unwrap_or(branch);
                return Some(RefLabel {
                    kind: RefKind::Head,
                    name: name.to_string(),
                });
            }
            // 游离 HEAD（detached）。
            if token == "HEAD" {
                return Some(RefLabel {
                    kind: RefKind::Head,
                    name: "HEAD".to_string(),
                });
            }
            if let Some(tag) = token.strip_prefix("refs/tags/") {
                return Some(RefLabel {
                    kind: RefKind::Tag,
                    name: tag.to_string(),
                });
            }
            if let Some(remote) = token.strip_prefix("refs/remotes/") {
                // 隐藏远程的符号 HEAD（如 origin/HEAD），它对历史浏览无意义。
                if remote.ends_with("/HEAD") {
                    return None;
                }
                return Some(RefLabel {
                    kind: RefKind::RemoteBranch,
                    name: remote.to_string(),
                });
            }
            if let Some(local) = token.strip_prefix("refs/heads/") {
                return Some(RefLabel {
                    kind: RefKind::LocalBranch,
                    name: local.to_string(),
                });
            }
            // 其它未知装饰（如 grafted / replaced）忽略。
            None
        })
        .collect()
}

/// 加载某仓库的提交图谱。
///
/// `limit` 控制本次最多取多少提交，`skip` 用于分页（已加载数）。
/// 使用 `--all` 覆盖所有引用、`--date-order` 保证布局稳定、`--decorate=full`
/// 让 `%D` 可被 [`parse_decorate`] 可靠区分类别。
#[cfg(not(target_family = "wasm"))]
pub(crate) async fn load_commit_graph(
    repo_root: &Path,
    limit: usize,
    skip: usize,
) -> Result<Vec<CommitNode>> {
    let n = limit.to_string();
    let skip_s = skip.to_string();
    let args = [
        "log",
        "--all",
        "--date-order",
        "--decorate=full",
        "--no-color",
        "-n",
        &n,
        "--skip",
        &skip_s,
        LOG_FORMAT,
    ];
    let stdout = warp_util::git::run_git_command(repo_root, &args).await?;
    Ok(parse_commit_log(&stdout))
}

/// 一个提交涉及的单个变更文件及其增删行数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChangedFile {
    pub path: String,
    pub additions: u32,
    pub deletions: u32,
}

/// 选中提交的详情：committer 信息、完整提交信息、变更文件列表。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommitDetail {
    pub committer_name: String,
    pub committer_time: i64,
    /// 完整提交信息（`%B`，含标题与正文）。
    pub message: String,
    pub files: Vec<ChangedFile>,
}

/// 加载单个提交的详情。
#[cfg(not(target_family = "wasm"))]
pub(crate) async fn load_commit_detail(repo_root: &Path, hash: &str) -> Result<CommitDetail> {
    // 用 `%x1e` 把 format 头部与随后的 `--numstat` 行分隔开。
    let args = [
        "show",
        "--numstat",
        "--no-color",
        "--format=%cn%x1f%ct%x1f%B%x1e",
        hash,
    ];
    let stdout = warp_util::git::run_git_command(repo_root, &args).await?;
    Ok(parse_commit_detail(&stdout))
}

/// 解析 `git show --numstat --format=%cn%x1f%ct%x1f%B%x1e` 的输出。
pub(crate) fn parse_commit_detail(stdout: &str) -> CommitDetail {
    let (header, numstat) = stdout.split_once(RECORD_SEP).unwrap_or((stdout, ""));
    let mut fields = header.splitn(3, UNIT_SEP);
    let committer_name = fields.next().unwrap_or("").to_string();
    let committer_time = fields
        .next()
        .and_then(|s| s.trim().parse::<i64>().ok())
        .unwrap_or(0);
    let message = fields.next().unwrap_or("").trim().to_string();

    CommitDetail {
        committer_name,
        committer_time,
        message,
        files: parse_numstat(numstat),
    }
}

/// 解析 `--numstat` 输出（每行 `additions\tdeletions\tpath`；二进制文件为 `-`）。
fn parse_numstat(stdout: &str) -> Vec<ChangedFile> {
    stdout
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let mut parts = line.splitn(3, '\t');
            let additions = parts.next()?;
            let deletions = parts.next()?;
            let path = parts.next()?.to_string();
            Some(ChangedFile {
                path,
                // 二进制文件的列为 "-"，按 0 处理。
                additions: additions.parse::<u32>().unwrap_or(0),
                deletions: deletions.parse::<u32>().unwrap_or(0),
            })
        })
        .collect()
}

#[cfg(test)]
#[path = "data_tests.rs"]
mod tests;
