//! Git Graph 的数据层：提交数据类型、`git log` 输出解析（纯函数）与异步取数。
//!
//! 取数统一走 [`warp_util::git::run_git_command`]（shell 出 `git`），不依赖
//! `git2`/`gix`。解析逻辑与 IO 分离：`parse_*` 是纯函数，可独立单测；
//! `load_*` 只做"组装命令 + 调用解析"的薄封装。

#[cfg(not(target_family = "wasm"))]
use std::path::{Path, PathBuf};

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
/// `branch_refs` 控制图谱覆盖范围：`None` 用 `--all`（所有引用）；`Some(refs)` 只覆盖给定
/// 的分支 ref（如 `refs/heads/main`、`refs/remotes/origin/dev`），空切片表示用户取消了全部
/// 分支，返回空图谱。`limit`/`skip` 用于分页。`--date-order` 保证布局稳定、`--decorate=full`
/// 让 `%D` 可被 [`parse_decorate`] 可靠区分类别。
#[cfg(not(target_family = "wasm"))]
pub(crate) async fn load_commit_graph(
    repo_root: &Path,
    branch_refs: Option<&[String]>,
    limit: usize,
    skip: usize,
) -> Result<Vec<CommitNode>> {
    let n = limit.to_string();
    let skip_s = skip.to_string();
    let mut args: Vec<&str> = vec![
        "log",
        "--date-order",
        "--decorate=full",
        "--no-color",
        "-n",
        &n,
        "--skip",
        &skip_s,
        LOG_FORMAT,
    ];
    // 选定的分支 ref 作为 revision 跟在 options 之后；None 时退回 --all。
    match branch_refs {
        None => args.push("--all"),
        Some(refs) => {
            if refs.is_empty() {
                return Ok(Vec::new());
            }
            args.extend(refs.iter().map(String::as_str));
        }
    }
    let stdout = warp_util::git::run_git_command(repo_root, &args).await?;
    Ok(parse_commit_log(&stdout))
}

/// 一个可用于过滤图谱的分支引用。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BranchRef {
    /// 展示名（去掉 `refs/heads/` 或 `refs/remotes/` 前缀）。
    pub display_name: String,
    /// 传给 `git log` 的完整 ref（如 `refs/heads/main`、`refs/remotes/origin/main`）。
    pub ref_name: String,
    /// 本地分支还是远程分支（决定展示分组/样式）。
    pub kind: RefKind,
}

/// 加载某仓库的本地 + 远程分支列表（供分支过滤下拉使用）。
#[cfg(not(target_family = "wasm"))]
pub(crate) async fn load_branches(repo_root: &Path) -> Result<Vec<BranchRef>> {
    let args = [
        "for-each-ref",
        "--format=%(refname)",
        "refs/heads",
        "refs/remotes",
    ];
    let stdout = warp_util::git::run_git_command(repo_root, &args).await?;
    Ok(parse_branch_refs(&stdout))
}

/// 解析 `git for-each-ref --format=%(refname)` 的输出为分支列表。
///
/// 每行是一个完整 ref。过滤掉远程符号 HEAD（`refs/remotes/*/HEAD`，对浏览无意义）。
/// 本地分支在前、远程分支在后，各自按名排序，保证下拉顺序稳定。
pub(crate) fn parse_branch_refs(stdout: &str) -> Vec<BranchRef> {
    let mut locals = Vec::new();
    let mut remotes = Vec::new();
    for line in stdout.lines() {
        let full = line.trim();
        if let Some(name) = full.strip_prefix("refs/heads/") {
            locals.push(BranchRef {
                display_name: name.to_string(),
                ref_name: full.to_string(),
                kind: RefKind::LocalBranch,
            });
        } else if let Some(name) = full.strip_prefix("refs/remotes/") {
            if name.ends_with("/HEAD") {
                continue;
            }
            remotes.push(BranchRef {
                display_name: name.to_string(),
                ref_name: full.to_string(),
                kind: RefKind::RemoteBranch,
            });
        }
    }
    locals.sort_by(|a, b| a.display_name.cmp(&b.display_name));
    remotes.sort_by(|a, b| a.display_name.cmp(&b.display_name));
    locals.extend(remotes);
    locals
}

/// 发现 `anchor` 相关的所有 git 仓库根，按展示顺序返回（去重）：
/// 1. 锚点自身所属仓库：用 `git rev-parse --show-toplevel` 向上探——锚点可能是某仓库的
///    子目录（如终端 `cd` 进了 `repo/crates`），这一步保留"在子目录里也能看父仓库历史"的行为，
///    且作为列表第一项（最贴近用户当前所在位置）。
/// 2. 第 1..=`depth` 层子目录里的仓库：见 [`scan_subdir_repos`]。
///
/// 用于"一个目录下挂着多个独立 git 项目"的场景（如把 `~/Projects` 作为工作目录）。
#[cfg(not(target_family = "wasm"))]
pub(crate) async fn discover_repositories(anchor: &Path, depth: usize) -> Vec<PathBuf> {
    let mut repos: Vec<PathBuf> = Vec::new();

    // 锚点自身所属仓库（向上探）。失败（不在任何仓库内）则跳过，不报错。
    if let Ok(stdout) =
        warp_util::git::run_git_command(anchor, &["rev-parse", "--show-toplevel"]).await
    {
        let toplevel = stdout.trim();
        if !toplevel.is_empty() {
            repos.push(PathBuf::from(toplevel));
        }
    }

    // 子目录里的独立仓库（与锚点所属仓库去重）。
    for repo in scan_subdir_repos(anchor, depth) {
        if !repos.contains(&repo) {
            repos.push(repo);
        }
    }

    repos
}

/// 扫描 `anchor` 的第 1..=`depth` 层子目录，返回其中带 `.git` 标记（仓库根）的目录，按路径排序。
///
/// 语义：`anchor` 自身是第 0 层，其直接子目录是第 1 层。`depth==0` 时不扫描任何子目录。
/// 命中一个仓库根后**不再深入其内部**——避免把它的 submodule / 嵌套仓库当作并列的独立项目。
#[cfg(not(target_family = "wasm"))]
fn scan_subdir_repos(anchor: &Path, depth: usize) -> Vec<PathBuf> {
    use std::collections::VecDeque;

    let mut found: Vec<PathBuf> = Vec::new();
    // BFS 队列：(目录, 该目录所处层级)。
    let mut queue: VecDeque<(PathBuf, usize)> = VecDeque::new();
    queue.push_back((anchor.to_path_buf(), 0));

    while let Some((dir, level)) = queue.pop_front() {
        // 已到达深度上限：该层目录不再展开其子目录。
        if level >= depth {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            // 只看目录（忽略普通文件；`.git` 既可能是目录也可能是文件，用 exists 判定）。
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let path = entry.path();
            if path.join(".git").exists() {
                // 命中仓库根：收录，且不入队其子目录（不深入仓库内部）。
                found.push(path);
            } else {
                // 普通目录：继续向下一层扫描。
                queue.push_back((path, level + 1));
            }
        }
    }

    // read_dir 顺序依赖文件系统，排序保证列表稳定（UI 下拉顺序、测试可复现）。
    found.sort();
    found
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

/// 选中提交对某个文件的改动（`commit~1..commit`，即"该提交自身改了什么"）：
/// 父版本的完整文件内容（作为 diff base）+ 解析好的 unified diff hunks。
/// 供"点击提交详情里的变更文件 → 主区只读 diff pane"使用。
#[cfg(not(target_family = "wasm"))]
#[derive(Debug, Clone)]
pub(crate) struct CommitFileDiff {
    /// 文件在父提交处的完整内容；新增文件或根提交无父版本时为空串（此时 diff 全为新增行）。
    pub base_content: String,
    /// 该提交对此文件的 unified diff hunks（复用 code review 的解析器与类型）。
    pub hunks: Vec<crate::code_review::diff_state::DiffHunk>,
}

/// 加载某提交对单个文件的改动。`path` 为仓库相对路径。
///
/// 取数策略兼顾 普通 / 新增 / 删除 / 根 / 合并 提交：
/// - base 内容：`git show <hash>~1:<path>`；取不到（新增文件或根提交无父版本）按空串处理。
/// - diff：优先 `git diff <hash>~1 <hash> -- <path>`（对合并提交也能拿到"对比第一父"的常规 hunks，
///   避免 `git show` 合并提交输出的 combined diff `@@@` 无法解析）；`<hash>~1` 不存在（根提交）时
///   退回 `git show <hash> --format= -- <path>`（整文件按新增展示）。
#[cfg(not(target_family = "wasm"))]
pub(crate) async fn load_file_diff_at_commit(
    repo_root: &Path,
    hash: &str,
    path: &str,
) -> Result<CommitFileDiff> {
    use crate::code_review::diff_state::LocalDiffStateModel;

    let parent = format!("{hash}~1");

    // base：父版本完整内容；新增文件 / 根提交无父版本 → git 出错，按空串处理。
    let base_spec = format!("{parent}:{path}");
    let base_content = warp_util::git::run_git_command(repo_root, &["show", base_spec.as_str()])
        .await
        .unwrap_or_default();

    // diff：先按"对比第一父"取常规两路 diff；根提交无父则退回整文件新增。
    let diff_output = match warp_util::git::run_git_command(
        repo_root,
        &["diff", "--no-color", parent.as_str(), hash, "--", path],
    )
    .await
    {
        Ok(out) => out,
        Err(_) => {
            warp_util::git::run_git_command(
                repo_root,
                &["show", "--no-color", "--format=", hash, "--", path],
            )
            .await?
        }
    };

    let hunks = LocalDiffStateModel::parse_diff_hunks(&diff_output)?;
    Ok(CommitFileDiff {
        base_content,
        hunks,
    })
}

#[cfg(test)]
#[path = "data_tests.rs"]
mod tests;
