"""Fails unless every non-merge commit in BASE..HEAD carries a Signed-off-by line for its author's email (DCO)."""
import re
import subprocess
import sys

BOTS = ("[bot]@users.noreply.github.com",)


def commits(base: str, head: str) -> list[str]:
    out = subprocess.run(
        ["git", "rev-list", "--no-merges", f"{base}..{head}"], check=True, capture_output=True, text=True
    ).stdout
    return out.split()


def field(sha: str, fmt: str) -> str:
    # `git log`, not `git show -s`: the workflow's clone has commits without their trees, and `git show` fetches the
    # commit's tree from GitHub before it prints anything.
    return subprocess.run(
        ["git", "log", "-1", f"--format={fmt}", sha], check=True, capture_output=True, text=True
    ).stdout


def signed_off(message: str, email: str) -> bool:
    emails = re.findall(r"^Signed-off-by: .+ <([^>]+)>\s*$", message, re.MULTILINE)
    return email.lower() in (e.lower() for e in emails)


def main(base: str, head: str) -> int:
    missing = []
    for sha in commits(base, head):
        email = field(sha, "%ae").strip()
        if email.endswith(BOTS):
            continue
        if not signed_off(field(sha, "%B"), email):
            missing.append(f"{sha[:10]} {field(sha, '%s').strip()} (author {email})")
    if missing:
        print("These commits have no Signed-off-by line for their author:")
        print("\n".join(f"  {m}" for m in missing))
        print("Sign off with `git commit -s` (or `git rebase --signoff origin/main`); see CONTRIBUTING.md, 'Licence and DCO'.")
        return 1
    print("Every commit is signed off.")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1], sys.argv[2]))
