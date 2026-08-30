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
- no force pushes, deletions, or bypasses.

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
