# Repository security settings

GitHub settings are part of the production control surface. The desired state
is checked in under `repository-policy/` and applied through
`scripts/github-repository-policy.sh`.

Configure the default branch ruleset to require:

- pull requests with one approving review;
- review by Code Owners;
- dismissal of stale approvals after new commits;
- approval of the latest reviewable push;
- an additional approval for unattributed Copilot pull requests;
- conversation resolution;
- signed commits and linear history;
- all current CI, dependency review, security analysis, and CodeQL checks;
- branches to be up to date before merge;
- no force pushes or deletions.

The admin repository role is the one bypass actor, and it bypasses always.
This is a deliberate concession to a single-maintainer repository, not an
oversight. `CODEOWNERS` names one owner, and GitHub does not let anyone approve
their own pull request, so the one-approving-review rule is unsatisfiable by
the only person able to satisfy it: every pull request would deadlock. The
bypass was added to merge #4 and is recorded here rather than left as
undocumented drift between the hosted ruleset and this directory.

What the bypass does **not** relax: it is a merge-time override for an admin
only. Required status checks, signed commits, linear history, and conversation
resolution remain in force for every other actor, and CI remains the real gate
— an admin merging past review still cannot merge past a red build without
consciously choosing `--admin`. Grant the admin role to a second person and
this concession should be revisited, because at that point review is
satisfiable and the deadlock argument no longer holds.

The policy also enables secret scanning, push protection, Dependabot alerts,
and Dependabot security updates. It restricts GitHub Actions to the exact
actions used by this repository and requires every action reference to be a
full commit SHA. The default workflow token is read-only and cannot approve
pull requests.

GitHub's additional-approval control is in public preview and is returned by
the ruleset API even though it is not yet present in the published request
schema. The policy audit verifies that the secure default remains enabled. It
also verifies that the beta team-specific reviewer collection remains empty,
because team reviewers are unavailable for user-owned repositories.

Check the live settings without changing them:

```bash
just github-policy-check
```

Applying the policy changes GitHub-hosted settings and requires repository
administrator access:

```bash
just github-policy-apply
```

GitHub limits
[generic pattern scanning](https://docs.github.com/en/code-security/how-tos/secure-your-secrets/detect-secret-leaks/enabling-secret-scanning-for-generic-patterns),
exposed by the REST API as `secret_scanning_non_provider_patterns`, to
organization-owned repositories with GitHub Secret Protection. GitHub also
exposes secret validity-check configuration through organization-level
code-security configurations, not the repository REST endpoint used for this
personal repository. These controls are outside this repository's automated
policy; confirm their availability in the GitHub security settings UI during
the release audit.

Audit these settings before every production release and whenever a required
check, repository owner, or workflow name changes.
