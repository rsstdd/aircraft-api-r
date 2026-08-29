#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly SCRIPT_DIR
REPOSITORY_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
readonly REPOSITORY_ROOT
readonly POLICY_DIR="${REPOSITORY_ROOT}/.github/repository-policy"
readonly RULESET_NAME="production-main"
readonly API_VERSION="2022-11-28"
# GitHub resolves composite-action dependencies before evaluating their step conditions.
# Keep this list aligned with the pinned actions' manifests.
readonly -a TRANSITIVE_WORKFLOW_ACTIONS=(
  "github/codeql-action/upload-sarif@7188fc363630916deb702c7fdcf4e481b751f97a"
)

usage() {
  echo "Usage: $0 --local|--check|--apply" >&2
}

if [[ $# -ne 1 || ( $1 != "--local" && $1 != "--check" && $1 != "--apply" ) ]]; then
  usage
  exit 2
fi

readonly MODE="$1"

if ! command -v jq >/dev/null 2>&1; then
  echo "jq is required" >&2
  exit 2
fi

readonly -a GH_API=(
  gh api
  --hostname github.com
  --header "Accept: application/vnd.github+json"
  --header "X-GitHub-Api-Version: ${API_VERSION}"
)

github_api() {
  "${GH_API[@]}" "$@"
}

check_local_policy() {
  local file
  local action
  local pattern
  local matched
  local -a workflow_actions
  local -a workflow_job_names
  local -a allowed_patterns
  local -a required_status_names

  for file in "${POLICY_DIR}"/*.json; do
    jq empty "${file}" >/dev/null
  done

  mapfile -t workflow_actions < <(
    {
      sed -nE \
        's/^[[:space:]]*(-[[:space:]]+)?uses:[[:space:]]+([^[:space:]#]+).*/\2/p' \
        "${REPOSITORY_ROOT}"/.github/workflows/*.yml
      printf '%s\n' "${TRANSITIVE_WORKFLOW_ACTIONS[@]}"
    } | sort --unique
  )
  mapfile -t allowed_patterns < <(
    jq --raw-output '.patterns_allowed[]' "${POLICY_DIR}/selected-actions.json" \
      | sort --unique
  )
  mapfile -t workflow_job_names < <(
    sed -nE 's/^    name:[[:space:]]+(.+)$/\1/p' \
      "${REPOSITORY_ROOT}"/.github/workflows/*.yml \
      | sort --unique
  )
  mapfile -t required_status_names < <(
    jq --raw-output '
      .rules[]
      | select(.type == "required_status_checks")
      | .parameters.required_status_checks[].context
      | if test("^CodeQL \\((actions|rust)\\)$") then
          "CodeQL (${{ matrix.language }})"
        else
          .
        end
    ' "${POLICY_DIR}/main-ruleset.json" \
      | sort --unique
  )

  if ! diff --unified \
    <(printf '%s\n' "${workflow_job_names[@]}") \
    <(printf '%s\n' "${required_status_names[@]}"); then
    echo "required status checks do not match the workflow job names" >&2
    return 1
  fi

  for action in "${workflow_actions[@]}"; do
    if [[ ! ${action} =~ ^[^@]+@[0-9a-f]{40}$ ]]; then
      echo "workflow action is not pinned to a full commit SHA: ${action}" >&2
      return 1
    fi
    matched=false
    for pattern in "${allowed_patterns[@]}"; do
      if [[ ${action} == "${pattern}" ]]; then
        matched=true
        break
      fi
    done
    if [[ ${matched} == false ]]; then
      echo "workflow action is absent from the hosted allowlist: ${action}" >&2
      return 1
    fi
  done

  for pattern in "${allowed_patterns[@]}"; do
    matched=false
    for action in "${workflow_actions[@]}"; do
      if [[ ${action} == "${pattern}" ]]; then
        matched=true
        break
      fi
    done
    if [[ ${matched} == false ]]; then
      echo "hosted allowlist pattern is unused by every workflow: ${pattern}" >&2
      return 1
    fi
  done
}

normalize_ruleset() {
  jq --sort-keys '
    {
      name,
      target,
      enforcement,
      bypass_actors: (.bypass_actors // []),
      conditions,
      rules: (
        .rules
        | map(
            if .type == "required_status_checks" then
              .parameters.required_status_checks |= sort_by(.context)
            elif .type == "pull_request" then
              # These preview fields are returned but are not in the request schema.
              .parameters |= del(
                .require_extra_approval_for_unattributed_changes,
                .required_reviewers
              )
            else
              .
            end
          )
        | sort_by(.type)
      )
    }
  '
}

compare_json() {
  local label="$1"
  local expected_file="$2"
  local actual_json="$3"
  local jq_filter="$4"

  if ! diff --unified \
    <(jq --sort-keys "${jq_filter}" "${expected_file}") \
    <(jq --sort-keys "${jq_filter}" <<<"${actual_json}"); then
    echo "${label} does not match the checked-in policy" >&2
    return 1
  fi
}

ruleset_id() {
  local rulesets="$1"
  local count
  count="$(jq --arg name "${RULESET_NAME}" '[.[] | select(.name == $name)] | length' <<<"${rulesets}")"
  if [[ ${count} -gt 1 ]]; then
    echo "more than one ${RULESET_NAME} ruleset exists" >&2
    return 1
  fi
  jq --raw-output --arg name "${RULESET_NAME}" \
    '.[] | select(.name == $name) | .id' <<<"${rulesets}"
}

apply_policy() {
  local rulesets="$1"
  local id

  github_api --method PUT "repos/${REPOSITORY}/actions/permissions" \
    --input "${POLICY_DIR}/actions-permissions.json" >/dev/null
  github_api --method PUT "repos/${REPOSITORY}/actions/permissions/selected-actions" \
    --input "${POLICY_DIR}/selected-actions.json" >/dev/null
  github_api --method PUT "repos/${REPOSITORY}/actions/permissions/workflow" \
    --input "${POLICY_DIR}/workflow-permissions.json" >/dev/null
  github_api --method PATCH "repos/${REPOSITORY}" \
    --input "${POLICY_DIR}/security-and-analysis.json" >/dev/null
  github_api --method PUT "repos/${REPOSITORY}/vulnerability-alerts" >/dev/null
  github_api --method PUT "repos/${REPOSITORY}/automated-security-fixes" >/dev/null

  id="$(ruleset_id "${rulesets}")"
  if [[ -n ${id} ]]; then
    github_api --method PUT "repos/${REPOSITORY}/rulesets/${id}" \
      --input "${POLICY_DIR}/main-ruleset.json" >/dev/null
  else
    github_api --method POST "repos/${REPOSITORY}/rulesets" \
      --input "${POLICY_DIR}/main-ruleset.json" >/dev/null
  fi
}

check_policy() {
  local actions
  local selected_actions
  local workflow
  local repository
  local rulesets
  local id
  local ruleset

  actions="$(github_api "repos/${REPOSITORY}/actions/permissions")"
  compare_json \
    "Actions permissions" \
    "${POLICY_DIR}/actions-permissions.json" \
    "${actions}" \
    '{enabled, allowed_actions, sha_pinning_required}'

  selected_actions="$(github_api "repos/${REPOSITORY}/actions/permissions/selected-actions")"
  compare_json \
    "selected Actions" \
    "${POLICY_DIR}/selected-actions.json" \
    "${selected_actions}" \
    '{github_owned_allowed, verified_allowed, patterns_allowed: (.patterns_allowed | sort)}'

  workflow="$(github_api "repos/${REPOSITORY}/actions/permissions/workflow")"
  compare_json \
    "workflow token permissions" \
    "${POLICY_DIR}/workflow-permissions.json" \
    "${workflow}" \
    '{default_workflow_permissions, can_approve_pull_request_reviews}'

  repository="$(github_api "repos/${REPOSITORY}")"
  compare_json \
    "security and analysis" \
    "${POLICY_DIR}/security-and-analysis.json" \
    "${repository}" \
    '{security_and_analysis: {secret_scanning: .security_and_analysis.secret_scanning, secret_scanning_push_protection: .security_and_analysis.secret_scanning_push_protection}}'

  github_api "repos/${REPOSITORY}/vulnerability-alerts" >/dev/null
  github_api "repos/${REPOSITORY}/automated-security-fixes" >/dev/null

  rulesets="$(github_api "repos/${REPOSITORY}/rulesets")"
  id="$(ruleset_id "${rulesets}")"
  if [[ -z ${id} ]]; then
    echo "${RULESET_NAME} ruleset is missing" >&2
    return 1
  fi
  ruleset="$(github_api "repos/${REPOSITORY}/rulesets/${id}")"
  if ! jq --exit-status '
    [.rules[] | select(.type == "pull_request")] as $rules
    | ($rules | length) == 1
      and $rules[0].parameters.require_extra_approval_for_unattributed_changes == true
      and (($rules[0].parameters.required_reviewers // []) | length) == 0
  ' <<<"${ruleset}" >/dev/null; then
    echo "pull-request preview protections do not match policy" >&2
    return 1
  fi
  if ! diff --unified \
    <(normalize_ruleset <"${POLICY_DIR}/main-ruleset.json") \
    <(normalize_ruleset <<<"${ruleset}"); then
    echo "${RULESET_NAME} ruleset does not match the checked-in policy" >&2
    return 1
  fi

  echo "GitHub repository policy is active and matches the checked-in configuration."
}

check_local_policy
if [[ ${MODE} == "--local" ]]; then
  echo "GitHub policy files, required checks, and workflow action pins are consistent."
  exit 0
fi

if ! command -v gh >/dev/null 2>&1; then
  echo "gh is required" >&2
  exit 2
fi
gh auth status --hostname github.com >/dev/null

REPOSITORY="${GITHUB_REPOSITORY:-$(gh repo view --json nameWithOwner --jq .nameWithOwner)}"
readonly REPOSITORY
if [[ ! ${REPOSITORY} =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]]; then
  echo "invalid GitHub repository name: ${REPOSITORY}" >&2
  exit 2
fi

rulesets="$(github_api "repos/${REPOSITORY}/rulesets")"
if [[ ${MODE} == "--apply" ]]; then
  apply_policy "${rulesets}"
fi
check_policy
