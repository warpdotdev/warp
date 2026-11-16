---
name: triage-vulnerabilities
description: Triage and remediate security vulnerabilities across Warp infrastructure. Checks Dependabot alerts (GitHub), GCP Artifact Registry container scanning, and Docker Scout for public images. Use when the user asks to check for vulnerabilities, triage CVEs, fix dependency issues, update base images, or remediate security alerts.
---

# triage-vulnerabilities

Triage and remediate security vulnerabilities across four sources: GitHub Dependabot, GCP container registry scanning, Docker Scout, and Linear security issues.

## Vulnerability Sources

### 1. Dependabot (GitHub)

Repos with Dependabot enabled: `warp-internal`, `warp-server`, `warp-terraform`, `session-sharing-server`.

Fetch open alerts:

```bash
# All open alerts for a repo (returns JSON array)
gh api /repos/warpdotdev/<repo>/dependabot/alerts?state=open

# Useful fields per alert:
#   .number, .state, .html_url
#   .dependency.package.name, .dependency.package.ecosystem, .dependency.manifest_path
#   .security_advisory.cve_id, .security_advisory.summary, .security_advisory.severity
#   .security_vulnerability.first_patched_version.identifier

# Summary view: CVE/GHSA, package, severity, fix version, manifest
gh api /repos/warpdotdev/<repo>/dependabot/alerts?state=open \
  --jq '.[] | [.number, .security_advisory.cve_id // .security_advisory.ghsa_id, .dependency.package.name, .security_advisory.severity, (.security_vulnerability.first_patched_version.identifier // "no fix"), .dependency.manifest_path] | @tsv'
```

### 2. GCP Artifact Registry Scanning

Internal service images in `us-east4` across two projects:
- **Production** (`astral-field-294621`): `warp-server`, `warp-server-jobs`, `warp-server-migrations`, `session-sharing-server`, `pgbouncer-rtc`
- **Staging** (`warp-server-staging`): same repos plus `cloud-run-source-deploy`

Scan steps:

```bash
# List images in a repo (get digest for vulnerability query)
gcloud artifacts docker images list \
  us-east4-docker.pkg.dev/<project>/<repo> \
  --include-tags --sort-by=~create_time --limit=5

# List vulnerabilities for a specific image
gcloud artifacts vulnerabilities list \
  "us-east4-docker.pkg.dev/<project>/<repo>/<image>@sha256:<digest>" \
  --format=json
```

Only scan the latest (most recently tagged) image per repo — older images are not actionable.

### 3. Docker Scout (Public Images)

Public images on Docker Hub under `warpdotdev/` org. Currently enrolled repos: `dev-base` (and potentially others — check with `docker scout repo list --org warpdotdev`).

```bash
# List CVEs (critical and high only)
docker scout cves warpdotdev/<image> --only-severity critical,high

# Check for base image update recommendations
docker scout recommendations warpdotdev/<image>
```

### 4. Linear Security Issues

Linear issues with the Security label track vulnerabilities that may not be auto-reported by other tools (e.g., internal findings, manual triages, or organizational security concerns). Note that Linear issues are not automatically closed when vulnerabilities are fixed elsewhere (e.g., via Dependabot PRs), so there may be duplicates across sources.

Find open security issues:

```bash
# Use the Linear MCP to search for issues with Security label
# Example query structure (use Linear MCP tool call):
#   - Search for issues with label: "Security"
#   - Filter for state: "Backlog", "Todo", "In Progress" (exclude "Done", "Cancelled")
#   - Return issue number, title, URL, and current status

# Useful fields per issue:
#   - Issue ID/number (e.g., CLD-2726)
#   - Title (usually contains CVE ID, e.g., "warp-server-GHSA-8r9q-7v3j-jr4g")
#   - URL (e.g., https://linear.app/warpdotdev/issue/CLD-2726/...)
#   - Status (Backlog, Todo, In Progress, Done, Cancelled)
#   - Description (contains CVE details and affected package info)
```

## Triage Workflow

### Step 1: Gather Vulnerabilities

**IMPORTANT**: Use TODOs to track each source.

Query all four sources. Write results to temporary files for reference:
- `dependabot_alerts.tsv` — CVE, package, severity, repo, fix version
- `gcp_vulns.tsv` — CVE, severity, package, image, fix available
- `scout_vulns.tsv` — CVE, severity, package, image
- `linear_security.tsv` — Issue ID, CVE/Title, status, URL

### Step 2: Deduplicate

The same CVE may appear across multiple sources (e.g. a base image vuln reported by both GCP scanning and Docker Scout, or a Dependabot alert duplicated as a Linear issue). Group by CVE ID and note all affected sources/images. When a Linear issue duplicates a vulnerability that's already being tracked (e.g., via Dependabot), note this and consider it resolved once the underlying fix is applied.

**IMPORTANT**: Add a TODO for each unique vulnerability.

### Step 3: Remediate (One Vulnerability at a Time)

Make sure that you've checked ALL sources and deduplicated vulnerabilities before remediating any.

For each unique vulnerability, starting with critical severity first:

#### a. Check for Upstream Fix

- **Dependabot alerts**: check `security_vulnerability.first_patched_version`
- **GCP scanning**: check the `FIX_AVAILABLE` field
- **Docker Scout**: check `docker scout recommendations` for base image updates
- **External sources** if needed:
  - NVD: `https://nvd.nist.gov/vuln/detail/<CVE-ID>`
  - GitHub Advisory Database: `https://github.com/advisories`
  - Distroless issues: `https://github.com/GoogleContainerTools/distroless/issues`

If no upstream fix exists, report the vulnerability and move on. Do NOT attempt to deactivate/dismiss alerts — only a human can do this.

#### b. Apply Fix

Fixes fall into three categories:

**Dependabot auto-fix available**: The simplest case. Check if Dependabot has already created a PR:

```bash
gh pr list --repo warpdotdev/<repo> --author app/dependabot --state open --json title,url
```

If a PR exists, review and approve it. If not, the fix may require manual intervention.

**Dependency update**: When Dependabot can't auto-fix (e.g. major version bump needed), update the dependency manually in the appropriate manifest (`Cargo.toml`, `go.mod`, `package.json`, etc.), run tests, and submit a PR.
- It's acceptable to update additional dependencies if necessary —  for example, if PackageA depends on a vulnerable version of PackageB, and also depends on PackageC, and the version of PackageA that uses a fixed version of PackageB requires a newer version of PackageC, you may update all three packages.
- Prefer updating direct dependencies rather than adding overrides (e.g. update `regex` to pull in a fixed version of `aho-corasick`).
- If there is not a version of the direct dependency that pulls in a fixed transitive dependency, and adding an override ist not straightforward, decide if it's best to wait -- consider the severity and surface area of the vulnerability.

**Infrastructure change**: For container image vulnerabilities, the fix may involve:
- Updating a base Docker image (e.g. in a `Dockerfile`)
- Updating a sidecar image version (e.g. OpenTelemetry collector in `session-sharing-server`)
- Waiting for a distroless base image update (not actionable — report and skip)

When submitting a fix PR, include:
- The CVE ID(s) being addressed in the PR title
- Links to the advisory in the PR description
- Use the `create-pr` skill for PR creation

#### c. Report Unresolvable Vulnerabilities

If a vulnerability has no available fix (e.g. waiting on distroless base image update), report:
- CVE ID and severity
- Affected package/image
- Why no fix is available
- Where to track the upstream fix (link to issue/advisory)

Suggest that a human deactivate the alert until a fix is available. NEVER dismiss or deactivate alerts yourself.

## Important Constraints

- **Never dismiss/deactivate vulnerability alerts** — only a human can do this
- **Work on one vulnerability at a time** — apply fix, verify, then move to the next
- **Prioritize by severity** — critical > high > medium > low
- **Check production project first** (`astral-field-294621`), then staging
- **For distroless images**: fixes depend on upstream base image updates, which we cannot control
- **Consider how the dependency is used**: vulnerabilities in build tools are less severe than vulnerabilities in packages that run in production
