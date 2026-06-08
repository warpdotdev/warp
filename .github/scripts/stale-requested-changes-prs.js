// Follows up on stale external-contributor PRs with active requested-changes
// reviews: posts escalating reminders, then closes after the final warning.
// Invoked from .github/workflows/stale_requested_changes_prs.yml via
// actions/github-script; `github` and `context` are injected by that action.
module.exports = async ({ github, context }) => {
  const { owner, repo } = context.repo;
  // Default to "full" if unset, because scheduled (cron) runs don't provide inputs
  const mode = (context.payload.inputs && context.payload.inputs.mode) || 'full';
  const canRemind = mode === 'reminder-only' || mode === 'full';
  const canClose = mode === 'full';

  const DAY_MS = 24 * 60 * 60 * 1000;
  const REMINDER_DAYS = [7, 14, 26];
  const FINAL_WARNING_DAY = 26;
  const CLOSE_DAY = 30;
  const EXTERNAL_LABEL = 'external-contributor';
  const EXEMPT_LABEL = 'no-autoclose';
  const BOT_LOGIN = 'github-actions[bot]';
  const markerFor = (stage) => `<!-- stale-requested-changes:stage=${stage} -->`;

  const now = Date.now();
  const ts = (value) => (value ? new Date(value).getTime() : 0);

  // A PR has active requested changes when any reviewer's latest *decisive*
  // review (APPROVED / CHANGES_REQUESTED / DISMISSED, tracked separately from
  // COMMENTED so a later comment-reply doesn't flip state) is CHANGES_REQUESTED.
  const hasActiveChangesRequested = (reviews) => {
    const decisive = new Map();
    const latest = new Map();
    for (const r of reviews) {
      const login = r.user && r.user.login;
      if (!login) continue;
      const at = ts(r.submitted_at);
      if (!latest.has(login) || at > latest.get(login).at) {
        latest.set(login, { state: r.state, at });
      }
      if (['APPROVED', 'CHANGES_REQUESTED', 'DISMISSED'].includes(r.state)) {
        if (!decisive.has(login) || at > decisive.get(login).at) {
          decisive.set(login, { state: r.state, at });
        }
      }
    }
    for (const login of latest.keys()) {
      const eff = decisive.get(login) || latest.get(login);
      if (eff.state === 'CHANGES_REQUESTED') return true;
    }
    return false;
  };

  const openPRs = await github.paginate(github.rest.pulls.list, {
    owner,
    repo,
    state: 'open',
    per_page: 100,
  });

  const summary = [];

  for (const pr of openPRs) {
    const labels = (pr.labels || []).map((l) => l.name);
    const author = pr.user && pr.user.login;
    if (pr.draft) continue;
    if (pr.user && pr.user.type === 'Bot') continue;
    // Skip internal member PRs and PRs with the no-autoclose label
    if (!labels.includes(EXTERNAL_LABEL)) continue;
    if (labels.includes(EXEMPT_LABEL)) continue;

    const reviews = await github.paginate(github.rest.pulls.listReviews, {
      owner,
      repo,
      pull_number: pr.number,
      per_page: 100,
    });
    if (!hasActiveChangesRequested(reviews)) continue;

    const [issueComments, reviewComments] = await Promise.all([
      github.paginate(github.rest.issues.listComments, { owner, repo, issue_number: pr.number, per_page: 100 }),
      github.paginate(github.rest.pulls.listReviewComments, { owner, repo, pull_number: pr.number, per_page: 100 }),
    ]);

    // Server-recorded head-branch push time. commit.pushedDate and the
    // force-push event timestamp are GitHub-controlled, so a backdated or
    // force-pushed commit still resets the timer (unlike commit author/committer
    // dates, which the contributor controls).
    const pushData = await github.graphql(
      `query($owner: String!, $repo: String!, $number: Int!) {
        repository(owner: $owner, name: $repo) {
          pullRequest(number: $number) {
            commits(last: 1) { nodes { commit { pushedDate committedDate } } }
            timelineItems(last: 50, itemTypes: [HEAD_REF_FORCE_PUSHED_EVENT]) {
              nodes { ... on HeadRefForcePushedEvent { createdAt } }
            }
          }
        }
      }`,
      { owner, repo, number: pr.number }
    );
    const prGraph = pushData.repository.pullRequest;
    const headCommit = (prGraph.commits.nodes[0] || {}).commit || {};

    // Author-driven last activity: PR creation, head-branch push, and comments
    // by the PR author. Maintainer/third-party/bot activity is ignored.
    let lastActivity = ts(pr.created_at);
    lastActivity = Math.max(lastActivity, ts(headCommit.pushedDate || headCommit.committedDate));
    for (const ev of prGraph.timelineItems.nodes) {
      lastActivity = Math.max(lastActivity, ts(ev.createdAt));
    }
    for (const c of issueComments) {
      if (c.user && c.user.login === author) lastActivity = Math.max(lastActivity, ts(c.created_at));
    }
    for (const c of reviewComments) {
      if (c.user && c.user.login === author) lastActivity = Math.max(lastActivity, ts(c.created_at));
    }

    const inactiveDays = (now - lastActivity) / DAY_MS;

    // A stage counts as sent only when its marker comment was posted by our own
    // workflow identity in the current window (at/after the last activity).
    const sentStages = new Set();
    for (const c of issueComments) {
      if (!c.user || c.user.login !== BOT_LOGIN) continue;
      if (ts(c.created_at) < lastActivity) continue;
      for (const stage of REMINDER_DAYS) {
        if ((c.body || '').includes(markerFor(stage))) sentStages.add(stage);
      }
    }

    const dueStage = [...REMINDER_DAYS].reverse().find((s) => inactiveDays >= s);
    const finalWarningSent = sentStages.has(FINAL_WARNING_DAY);
    const shouldClose = inactiveDays >= CLOSE_DAY && finalWarningSent;

    let action = 'none';
    if (shouldClose) action = 'close';
    else if (dueStage !== undefined && !sentStages.has(dueStage)) action = `remind:${dueStage}`;

    const days = Math.floor(inactiveDays);
    console.log(`PR #${pr.number} (@${author}, inactive ${days}d): ${action}${action === 'none' ? '' : ` [mode=${mode}]`}`);
    if (action === 'none') continue;
    summary.push(`#${pr.number} ${action} (${days}d)`);

    if (action === 'close') {
      if (!canClose) continue;
      await github.rest.issues.createComment({
        owner,
        repo,
        issue_number: pr.number,
        body: `Closing this pull request because the requested changes have gone unaddressed for over ${CLOSE_DAY} days. If you'd like to continue, push your updates and reopen the PR (or comment to ask a maintainer to reopen) — we'd be glad to pick it back up.`,
      });
      await github.rest.pulls.update({ owner, repo, pull_number: pr.number, state: 'closed' });
      continue;
    }

    if (!canRemind) continue;
    const stage = Number(action.split(':')[1]);
    const remaining = Math.max(1, CLOSE_DAY - days);
    const body = stage === FINAL_WARNING_DAY
      ? `Hi @${author} — final reminder: a reviewer requested changes on this PR and it has been inactive for ${days} days. It will be **automatically closed in about ${remaining} day(s)** unless you push updates or reply. Maintainers can apply the \`${EXEMPT_LABEL}\` label to keep it open.\n\n${markerFor(stage)}`
      : `Hi @${author} — a reviewer requested changes on this PR and it hasn't had activity from you in ${days} days. When you get a chance, please push updates or reply to the review so a reviewer can take another look. Without activity, this PR will be automatically closed after ${CLOSE_DAY} days of inactivity.\n\n${markerFor(stage)}`;
    await github.rest.issues.createComment({ owner, repo, issue_number: pr.number, body });
  }

  console.log(`mode=${mode}; acted on ${summary.length} PR(s): ${summary.join(', ') || 'none'}`);
};
