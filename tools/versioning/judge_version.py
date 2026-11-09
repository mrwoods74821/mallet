"""Pick the next Mallet release version from the commits since the last release, using standard SemVer.

Run by the Release workflow (.github/workflows/release.yml). Every commit since the newest vMAJOR.MINOR.PATCH tag
is judged by the Copilot CLI, which picks one bump for it:

* major: a breaking change for mappers or for code that uses Mallet, such as a changed .mallet format, a
  renamed MCP tool or a changed argument. While the major version is 0, a breaking change raises the minor
  version instead, as SemVer allows for 0.y.z releases.
* minor: a new backward-compatible feature.
* patch: a backward-compatible fix or polish.
* none: nothing a mapper or an AI agent can notice, such as CI, documentation or agent notes.

The release takes the largest verdict among its commits, so a batch of work becomes one version step. A run whose
commits are all none releases nothing. Without a reachable Copilot, each commit is guessed from its subject.

The script only rewrites the version in Cargo.toml and writes the release notes, the step summary and a KEY=value
result file. Committing, tagging, pushing and publishing are the workflow's job.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shlex
import subprocess
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path

BOT_EMAIL = "41898282+github-actions[bot]@users.noreply.github.com"
SEMVER_TAG = re.compile(r"^v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$")
VERSION_LINE = re.compile(r'^version = "(\d+)\.(\d+)\.(\d+)"(\r?)$', re.MULTILINE)
BUMPS = ("none", "patch", "minor", "major")
NON_APP_PREFIXES = (".github/", ".claude/", "docs/", "tools/versioning/")
NON_APP_FILES = ("AGENTS.md", "CLAUDE.md", "README.md", ".gitignore", ".gitattributes", "rustfmt.toml")
COPILOT_COMMAND = shlex.split(os.environ.get("COPILOT_COMMAND", "copilot"))
DEFAULT_MODEL = "auto"
# Copilot receives the prompt as one command-line argument, and Windows caps a command line at 32,767 characters.
# The instructions take about 4,000 more, so keep the commit text of one prompt well under the cap.
PROMPT_BUDGET = 20000
BODY_LIMIT = 2000
DIFF_LIMIT = 6000
STAT_LIMIT = 1500
ENTRY_LIMIT = 240
SECTIONS = (("additions", "Additions"), ("changes", "Changes"), ("fixes", "Bug fixes"))
SECTION_IDS = tuple(section for section, _ in SECTIONS)

RUBRIC = (
    "The repository is Mallet, a Hammer-style level editor for Source engine VMF maps, written in Rust. Its public "
    "surface is what mappers and AI agents rely on: the .mallet map format, VMF reading and writing that must not "
    "lose data, the entity and FGD handling, the compile presets, and the MCP tools an AI agent calls.\n"
    "- major: breaks existing work. A .mallet or saved-options format changes so old files no longer open; an MCP "
    "tool or its arguments are removed, renamed or change meaning; VMF output changes in a way that breaks compiles.\n"
    "- minor: adds something without breaking anything. A new tool, editor mode, option, compile preset, entity "
    "handling or MCP tool; a clearly new behaviour mappers will notice.\n"
    "- patch: fixes or polishes existing behaviour. Bug fixes, crash fixes, performance, wording, visual polish of "
    "an existing screen, internal refactors that change nothing outside.\n"
    "- none: nothing a mapper or AI agent can notice: CI workflows, agent notes, documentation-only edits, "
    "tooling."
)

CHANGELOG_RULES = (
    "- Each entry is one plain-English sentence ending with a full stop, under 200 characters. Start additions with "
    '"Added" and fixes with "Fixed". Write changes as what is true now, such as "Lockers now ..." or "Players can no '
    'longer ...".\n'
    "- Say what a mapper notices in the editor or in the compiled map, never how the code does it: no file, function, "
    "type or variable names, no commit hashes and no em dashes.\n"
    "- Check every entry against the diff so it says what the change really does.\n"
)

ANSWER_RULES = "Answer with one JSON object and nothing else, no prose and no code fence, in this form: "


@dataclass(frozen=True)
class Version:
    """A SemVer version, MAJOR.MINOR.PATCH."""

    major: int
    minor: int
    patch: int

    def __str__(self) -> str:
        return f"{self.major}.{self.minor}.{self.patch}"

    @property
    def tag(self) -> str:
        """The git tag for this version, prefixed with v."""
        return f"v{self}"

    def bump(self, kind: str, forced: bool = False) -> "Version":
        """Return the version one kind of step later. While the major version is 0, a breaking change raises the minor."""
        if kind == "major":
            if self.major == 0 and not forced:
                return Version(0, self.minor + 1, 0)

            return Version(self.major + 1, 0, 0)

        if kind == "minor":
            return Version(self.major, self.minor + 1, 0)

        if kind == "patch":
            return Version(self.major, self.minor, self.patch + 1)

        return self


@dataclass
class Commit:
    """A commit waiting for a verdict, with what the judge is shown about it."""

    sha: str
    subject: str
    body: str
    author_email: str
    files: list[str]
    stat: str = ""
    diff: str = ""
    bump: str = "none"
    reason: str = ""
    source: str = ""

    @property
    def short(self) -> str:
        """The seven-character abbreviation used in notes and the step summary."""
        return self.sha[:7]


@dataclass
class Entry:
    """One line of the release notes and the section it belongs to."""

    section: str
    text: str
    commits: list[str] = field(default_factory=list)


def git(*args: str, check: bool = True) -> str:
    """Run git with args and return its stripped stdout."""
    result = subprocess.run(["git", *args], capture_output=True, text=True, encoding="utf-8", errors="replace")
    if check and result.returncode != 0:
        raise RuntimeError(f"git {' '.join(args)} failed: {result.stderr.strip()}")

    return result.stdout.strip()


def read_version(path: Path) -> Version:
    """Read the version from the workspace Cargo.toml."""
    match = VERSION_LINE.search(path.read_bytes().decode("utf-8"))
    if match is None:
        raise RuntimeError(f'No version = "X.Y.Z" line found in {path}')

    return Version(int(match[1]), int(match[2]), int(match[3]))


def write_version(path: Path, version: Version) -> None:
    """Replace the workspace version line, keeping the rest of the file and its line endings as they are."""
    text = path.read_bytes().decode("utf-8")
    updated, count = VERSION_LINE.subn(lambda match: f'version = "{version}"{match[4]}', text, count=1)
    if count != 1:
        raise RuntimeError(f"Could not write the version into {path}")

    path.write_bytes(updated.encode("utf-8"))


def latest_release_tag() -> str | None:
    """Return the newest vMAJOR.MINOR.PATCH tag reachable from HEAD, or None before the first release."""
    for tag in git("tag", "--merged", "HEAD", "--sort=-v:refname").splitlines():
        if SEMVER_TAG.match(tag):
            return tag

    return None


def parse_tag(tag: str) -> Version:
    """Turn a vMAJOR.MINOR.PATCH tag back into a version."""
    match = SEMVER_TAG.match(tag)
    if match is None:
        raise RuntimeError(f"{tag} is not a vMAJOR.MINOR.PATCH tag")

    return Version(int(match[1]), int(match[2]), int(match[3]))


def collect_commits(tag: str) -> list[Commit]:
    """List every commit after tag, oldest first, skipping merges and the release bot's own commits."""
    shas = git("rev-list", "--reverse", "--no-merges", f"{tag}..HEAD").split()
    commits = []
    for sha in shas:
        header = git("show", "-s", "--format=%s%x1f%b%x1f%ae", sha).split("\x1f")
        author_email = header[2].strip() if len(header) > 2 else ""
        if author_email == BOT_EMAIL:
            continue

        files = [line for line in git("show", "--format=", "--name-only", sha).splitlines() if line]
        commits.append(Commit(
            sha=sha,
            subject=header[0].strip(),
            body=header[1].strip() if len(header) > 1 else "",
            author_email=author_email,
            files=files,
            stat=git("show", "--format=", "--stat=120", sha)[:STAT_LIMIT],
            diff=git("show", "--format=", "--no-color", "--no-ext-diff", "-U2", sha)[:DIFF_LIMIT],
        ))

    return commits


def is_non_app_file(path: str) -> bool:
    """True for files a mapper or agent never sees: CI, agent tooling, documentation and version tooling."""
    return path.startswith(NON_APP_PREFIXES) or path.split("/")[-1] in NON_APP_FILES


def judge_locally(commit: Commit) -> bool:
    """Settle commits that need no Copilot call, returning True when a verdict was set."""
    if commit.subject.lower().startswith("[skip]"):
        commit.bump, commit.reason, commit.source = "none", "Marked [skip]: agent or tooling change.", "rule"
        return True

    if commit.files and all(is_non_app_file(path) for path in commit.files):
        commit.bump, commit.reason, commit.source = "none", "Touches only CI, tooling or documentation files.", "rule"
        return True

    return False


def judge_by_heuristic(commit: Commit, why: str) -> None:
    """Guess a verdict from the subject when Copilot is unavailable."""
    subject = commit.subject.lower()
    is_addition = re.match(r"^(feat|add|introduce|create|support|allow)\b", subject) is not None
    commit.bump = "minor" if is_addition else "patch"
    commit.reason = f"Guessed from the subject because Copilot was unavailable ({why})."
    commit.source = "heuristic"


def describe_commit(commit: Commit) -> str:
    """Render one commit for the judge prompt."""
    body = commit.body[:BODY_LIMIT] or "(no body)"
    return (
        f"### Commit {commit.sha}\n"
        f"Subject: {commit.subject}\n"
        f"Body:\n{body}\n\n"
        f"Files changed:\n{commit.stat}\n\n"
        f"Diff (may be truncated):\n{commit.diff}\n"
    )


def build_batches(commits: list[Commit], budget: int) -> list[list[Commit]]:
    """Pack commits into prompts that stay under the size Copilot accepts in one call."""
    batches, current, size = [], [], 0
    for commit in commits:
        cost = len(describe_commit(commit).encode("utf-8"))
        if current and size + cost > budget:
            batches.append(current)
            current, size = [], 0

        current.append(commit)
        size += cost

    if current:
        batches.append(current)

    return batches


def build_prompt(batch: list[Commit]) -> str:
    """Ask Copilot for one verdict per commit and the release-note lines they earn."""
    instructions = (
        "You are the release manager for Mallet. Versions use standard SemVer, MAJOR.MINOR.PATCH. The commits below "
        "were pushed since the last release and ship together as one release. Do two jobs. Do not use any tools: "
        "everything you need is in this prompt.\n\n"
        "Job 1, verdicts. Judge each commit on its own and pick exactly one bump: major, minor, patch or none. Pick the "
        "smallest bump that is honest about the change. Read the diff to confirm the subject.\n"
        f"{RUBRIC}\n\n"
        "Job 2, release notes. Write the entries for this release's notes.\n"
        "- Put each entry in one section: additions for something new; changes for things that now look, behave or "
        "cost differently, including removals; fixes for bugs that now work.\n"
        f"{CHANGELOG_RULES}"
        "- Merge commits that make or finish the same change into one entry. Leave out every commit judged none.\n"
        "- Within each section, put what matters most first. Never invent details the commits do not support.\n\n"
        f"{ANSWER_RULES}"
        '{"judgments": [{"sha": "<full sha>", "bump": "major|minor|patch|none", "reason": "<one short sentence>"}], '
        '"changelog": [{"section": "additions|changes|fixes", "text": "<entry>", "commits": ["<full sha>"]}]}. '
        "Give one judgment per commit, in the order given. The changelog may be an empty list.\n\n"
    )
    return instructions + "\n".join(describe_commit(commit) for commit in batch)


def extract_answer(output: str) -> dict:
    """Pull the JSON object out of Copilot's answer, tolerating a code fence or stray prose around it."""
    first, last = output.find("{"), output.rfind("}")
    if first < 0 or last <= first:
        raise RuntimeError(f"Copilot answered without JSON: {output.strip()[:200]}")

    parsed = json.loads(output[first:last + 1])
    if not isinstance(parsed, dict):
        raise RuntimeError("Copilot's answer was not a JSON object")

    return parsed


def ask_copilot(model: str, prompt: str, workdir: Path) -> dict:
    """Ask the Copilot CLI one question with every tool switched off, retrying once."""
    command = [
        *COPILOT_COMMAND, "-p", prompt, "-s", "--no-ask-user", "--available-tools=",
        "--disable-builtin-mcps", "--no-custom-instructions",
    ]
    if model:
        command += ["--model", model]

    error = "no attempt made"
    for attempt in range(2):
        if attempt > 0:
            time.sleep(15)

        result = subprocess.run(
            command, cwd=workdir, capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=600,
        )
        if result.returncode != 0:
            error = f"copilot exited {result.returncode}: {(result.stderr or result.stdout).strip()[:300]}"
            continue

        try:
            answer = extract_answer(result.stdout)
        except (RuntimeError, ValueError) as parse_error:
            error = str(parse_error)
            continue

        if isinstance(answer.get("judgments"), list):
            return answer

        error = "Copilot's answer had no judgments list"

    raise RuntimeError(error)


def apply_judgments(batch: list[Commit], judgments: list) -> None:
    """Match Copilot's judgments to their commits by sha prefix, then by position."""
    by_sha = {}
    for entry in judgments:
        if isinstance(entry, dict) and isinstance(entry.get("sha"), str):
            by_sha[entry["sha"].strip().lower()[:7]] = entry

    for index, commit in enumerate(batch):
        entry = by_sha.get(commit.short)
        if entry is None and index < len(judgments) and isinstance(judgments[index], dict):
            entry = judgments[index]

        bump = str((entry or {}).get("bump", "")).strip().lower()
        if bump not in BUMPS:
            judge_by_heuristic(commit, "no usable verdict for this commit")
            continue

        commit.bump = bump
        commit.reason = str(entry.get("reason", "")).strip()
        commit.source = "copilot"


def clean_text(text: object) -> str:
    """Tidy one release-note line into a single capitalised sentence without list markers or em dashes."""
    line = re.sub(r"\s+", " ", str(text or "")).strip()
    line = re.sub(r"^(?:[-*+]\s+|#+\s*)", "", line)
    line = re.sub(r"\s*—\s*", ", ", line)
    if len(line) > ENTRY_LIMIT:
        line = line[:ENTRY_LIMIT].rsplit(" ", 1)[0].rstrip(",;:") + "..."

    if not line:
        return ""

    if line[-1] not in ".!?":
        line += "."

    return line[0].upper() + line[1:]


def parse_entries(raw: object, batch: list[Commit]) -> list[Entry]:
    """Keep the well-formed release-note entries from Copilot, matching their commits back by sha prefix."""
    if not isinstance(raw, list):
        return []

    known = {commit.short: commit.sha for commit in batch}
    entries = []
    for item in raw:
        if not isinstance(item, dict):
            continue

        section = str(item.get("section", "")).strip().lower()
        text = clean_text(item.get("text"))
        if section not in SECTION_IDS or not text:
            continue

        shas = item.get("commits") if isinstance(item.get("commits"), list) else []
        prefixes = [str(sha).strip().lower()[:7] for sha in shas]
        entries.append(Entry(section, text, [known[prefix] for prefix in prefixes if prefix in known]))

    return entries


def judge_commits(model: str, commits: list[Commit], workdir: Path, budget: int) -> list[Entry]:
    """Give every commit a verdict and draft the release notes: by rule, by Copilot, or from subjects."""
    pending = [commit for commit in commits if not judge_locally(commit)]
    entries: list[Entry] = []
    for batch in build_batches(pending, budget):
        try:
            answer = ask_copilot(model, build_prompt(batch), workdir)
        except (RuntimeError, OSError, ValueError, subprocess.SubprocessError) as error:
            print(f"Copilot could not judge, falling back to the subject heuristic: {error}", file=sys.stderr)
            for commit in batch:
                judge_by_heuristic(commit, str(error)[:120])

            continue

        apply_judgments(batch, answer["judgments"])
        entries += parse_entries(answer.get("changelog"), batch)

    described = {sha for entry in entries for sha in entry.commits}
    for commit in pending:
        if commit.source == "heuristic" and commit.bump != "none" and commit.sha not in described:
            section = "fixes" if commit.subject.lower().startswith("fix") else "changes"
            entries.append(Entry(section, clean_text(commit.subject), [commit.sha]))

    return entries


def release_bump(commits: list[Commit]) -> str:
    """The largest verdict among the commits, which is the single step this release takes."""
    return max((commit.bump for commit in commits), key=BUMPS.index, default="none")


def build_notes(entries: list[Entry], commits: list[Commit], previous: str, version: Version, forced_bump: str) -> str:
    """Write the release notes: the entries by section, then the commits for developers and a compare link."""
    lines = []
    if forced_bump != "auto":
        lines += [f"This release was started by hand as a {forced_bump} release.", ""]

    for section, title in SECTIONS:
        texts = [entry.text for entry in entries if entry.section == section]
        if texts:
            lines += [f"## {title}", *[f"- {text}" for text in texts], ""]

    if not entries:
        lines += ["This release changes nothing a mapper can see; it holds behind-the-scenes work only.", ""]

    count = f"{len(commits)} commit" if len(commits) == 1 else f"{len(commits)} commits"
    lines += ["<details>", f"<summary>{count} in this release</summary>", ""]
    lines += [f"- {commit.short} {commit.subject}" for commit in commits]
    lines += ["", "</details>", ""]

    server = os.environ.get("GITHUB_SERVER_URL", "https://github.com")
    repository = os.environ.get("GITHUB_REPOSITORY")
    if repository:
        lines.append(f"Full list of changes: {server}/{repository}/compare/{previous}...{version.tag}")

    return "\n".join(lines).strip() + "\n"


def build_summary(commits: list[Commit], previous: str, target: str, model: str, notes: str) -> str:
    """Write the step summary: every verdict and why it was reached, then the release notes."""
    lines = [
        f"### Version judge: {previous} to {target}",
        "",
        f"Model: `{model}`",
        "",
        "| Commit | Bump | Judged by | Reason |",
        "| --- | --- | --- | --- |",
    ]
    for commit in commits:
        subject = commit.subject.replace("|", "/")
        reason = commit.reason.replace("|", "/")
        lines.append(f"| `{commit.short}` {subject} | {commit.bump} | {commit.source} | {reason} |")

    if notes:
        lines += ["", "#### Release notes", "", notes]

    return "\n".join(lines) + "\n"


def write_result(path: Path, values: dict) -> None:
    """Write the KEY=value file the workflow reads after the script returns."""
    path.write_text("".join(f"{key}={value}\n" for key, value in values.items()), encoding="utf-8")


def append_summary(text: str) -> None:
    """Append to the Actions step summary when running in a workflow, otherwise print it."""
    summary_path = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary_path:
        with open(summary_path, "a", encoding="utf-8") as handle:
            handle.write(text)
    else:
        print(text)


def parse_arguments() -> argparse.Namespace:
    """Read the command line."""
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--cargo-toml", type=Path, default=Path("Cargo.toml"), help="Workspace Cargo.toml holding the version.")
    parser.add_argument("--notes", type=Path, required=True, help="Where to write the release notes.")
    parser.add_argument("--result", type=Path, required=True, help="Where to write the KEY=value result.")
    parser.add_argument("--model", default=DEFAULT_MODEL, help="Copilot model for the judge, or auto to let Copilot pick.")
    parser.add_argument("--forced-bump", choices=("auto", "patch", "minor", "major"), default="auto", help="Use one bump instead of the judged verdicts.")
    parser.add_argument("--workdir", type=Path, default=Path.cwd(), help="Empty directory Copilot runs in, so it loads no repository instructions.")
    parser.add_argument("--since", help="Judge the commits after this vMAJOR.MINOR.PATCH tag instead of the newest one.")
    parser.add_argument("--prompt-budget", type=int, default=PROMPT_BUDGET, help="Most bytes of commits sent in one Copilot prompt.")
    parser.add_argument("--dry-run", action="store_true", help="Judge and write the notes, but leave Cargo.toml alone and release nothing.")
    return parser.parse_args()


def main() -> int:
    """Seed the first release, or judge the commits since the last one and work out the next version and its notes."""
    args = parse_arguments()
    model = "" if (args.model or DEFAULT_MODEL) == "auto" else args.model
    previous = args.since or latest_release_tag()

    if previous is None:
        version = read_version(args.cargo_toml)
        args.notes.write_text(f"First tracked release, at version {version}.\n", encoding="utf-8")
        seed = "false" if args.dry_run else "true"
        write_result(args.result, {"RELEASE": seed, "SEED": seed, "VERSION": str(version), "TAG": version.tag})
        append_summary(f"### Version judge\n\nNo release tag yet, so HEAD is tagged {version.tag} as the starting point.\n")
        return 0

    base = parse_tag(previous)
    commits = collect_commits(previous)
    if not commits:
        write_result(args.result, {"RELEASE": "false", "SEED": "false", "VERSION": str(base), "TAG": previous})
        append_summary(f"### Version judge\n\nNo commits since {previous}; nothing to release.\n")
        return 0

    entries = judge_commits(model, commits, args.workdir, args.prompt_budget)

    is_forced = args.forced_bump != "auto"
    bump = args.forced_bump if is_forced else release_bump(commits)
    version = base.bump(bump, forced=is_forced)
    is_release = version != base

    notes = build_notes(entries, commits, previous, version, args.forced_bump) if is_release else ""
    target = f"{version.tag} ({bump})" if is_release else "no release"
    append_summary(build_summary(commits, previous, target, model or "auto", notes))

    if not is_release:
        write_result(args.result, {"RELEASE": "false", "SEED": "false", "VERSION": str(base), "TAG": previous})
        return 0

    if not args.dry_run:
        write_version(args.cargo_toml, version)

    args.notes.write_text(notes, encoding="utf-8")
    write_result(args.result, {
        "RELEASE": "false" if args.dry_run else "true", "SEED": "false", "VERSION": str(version), "TAG": version.tag,
    })
    return 0


if __name__ == "__main__":
    sys.exit(main())
