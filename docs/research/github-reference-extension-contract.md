# GitHub reference extension data contract

> Historical reference-shape research, superseded by ADR 0003. Production
> Extensions choose their own bounded HTTP requests and interpret arbitrary
> response bytes in one self-describing Lua script; there is no host-declared
> URL, transform, or template-facing response contract.

Status: recommended for the playlist/extension MVP
Research ticket: [#10](https://github.com/CorVous/WireTerm/issues/10)

## Decision

The reference GitHub extension should make one authenticated `POST https://api.github.com/graphql` request containing three aliased `search` connections, then use its transform to convert the GraphQL response into the small template-facing contract defined below.

Use these searches:

1. `is:open is:pr author:@me sort:updated-desc`
2. `is:open is:pr review-requested:@me sort:updated-desc`
3. `is:open is:issue assignee:@me sort:updated-desc`

`@me` makes the configuration account-independent. GitHub documents that username qualifiers accept `@me`; `review-requested` includes both direct requests and requests to one of the user's teams, and a request leaves the result after that user or team member reviews it. [Searching issues and pull requests](https://docs.github.com/en/search-github/searching-on-github/searching-issues-and-pull-requests#search-for-my-issues-and-pull-requests), [review qualifiers](https://docs.github.com/en/search-github/searching-on-github/searching-issues-and-pull-requests#search-by-pull-request-review-status-and-reviewer)

GraphQL is preferable to REST search for this extension because it:

- fetches all three lists in one read-only request;
- selects only fields the Liquid view needs;
- returns `repository.nameWithOwner` without parsing API URLs or issuing per-item repository lookups;
- gives each list its own count and cursor metadata; and
- exposes the request's rate-limit cost and remaining budget in the same response.

REST search remains a reasonable generic extension primitive, but this particular view would require three requests and would return large issue-shaped objects. REST search also caps every search at 1,000 results, returns at most 100 per page, has a separate authenticated limit of 30 requests per minute, and may mark timed-out results with `incomplete_results: true`. [REST search limits](https://docs.github.com/en/rest/search/search#about-search), [search issues and pull requests](https://docs.github.com/en/rest/search/search#search-issues-and-pull-requests)

## Authenticated request

Headers:

```http
Authorization: Bearer {{ secrets.github_token }}
Content-Type: application/json
User-Agent: WireTerm-GitHub-Reference-Extension
```

Body:

```json
{
  "query": "query GitHubDashboard($pageSize: Int!) {\n  viewer { login }\n  opened: search(query: \"is:open is:pr author:@me sort:updated-desc\", type: ISSUE, first: $pageSize) {\n    issueCount\n    pageInfo { hasNextPage endCursor }\n    nodes { ...PullRequestCard }\n  }\n  reviews: search(query: \"is:open is:pr review-requested:@me sort:updated-desc\", type: ISSUE, first: $pageSize) {\n    issueCount\n    pageInfo { hasNextPage endCursor }\n    nodes { ...PullRequestCard }\n  }\n  assigned: search(query: \"is:open is:issue assignee:@me sort:updated-desc\", type: ISSUE, first: $pageSize) {\n    issueCount\n    pageInfo { hasNextPage endCursor }\n    nodes { ...IssueCard }\n  }\n  rateLimit { cost remaining resetAt }\n}\nfragment PullRequestCard on PullRequest {\n  id\n  number\n  title\n  url\n  isDraft\n  updatedAt\n  repository { nameWithOwner }\n  author { login }\n  labels(first: 5) { nodes { name color } }\n}\nfragment IssueCard on Issue {\n  id\n  number\n  title\n  url\n  updatedAt\n  repository { nameWithOwner }\n  author { login }\n  labels(first: 5) { nodes { name color } }\n}",
  "variables": {
    "pageSize": 20
  }
}
```

The extension manifest should expose `page_size` as an integer setting with default `20`, minimum `1`, and maximum `50`. A 50-item ceiling is intentionally below GitHub's connection maximum and far beyond what an 800 × 480 frame can present. GitHub requires `first` or `last` on every connection and permits values from 1 through 100. [GraphQL pagination](https://docs.github.com/en/graphql/guides/using-pagination-in-the-graphql-api)

The transform must reject a `200` response containing a top-level non-empty `errors` array. GraphQL can return partial `data` alongside errors; silently rendering it would make a permission or schema failure look like an empty dashboard.

## Token permissions

Use a fine-grained personal access token for the MVP:

- repository access: only the repositories whose work may appear;
- repository permissions: **Issues: read-only** and **Pull requests: read-only**;
- account and organization permissions: none.

GitHub recommends fine-grained tokens when possible because they can be restricted to one resource owner, selected repositories, and specific permissions. Tokens cannot grant access the user does not already have. [Managing personal access tokens](https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/managing-your-personal-access-tokens#about-personal-access-tokens)

Two limitations must be explicit in setup guidance:

- a fine-grained token targets one user or organization, so a dashboard spanning repositories owned by multiple organizations needs one extension instance/token per resource owner in the MVP;
- an organization may require approval, and a pending token can read only public resources.

For public repositories only, GitHub's REST search endpoint requires no fine-grained permissions, but this contract deliberately declares the read permissions needed by the GraphQL fields rather than depending on endpoint-specific REST behavior. A classic PAT with no scopes can see public information; using classic `repo` to span private repositories is technically possible but grants much broader access and should not be the recommended setup. [REST search token requirements](https://docs.github.com/en/rest/search/search#fine-grained-access-tokens-for-search-issues-and-pull-requests), [classic-token scope behavior](https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/managing-your-personal-access-tokens#creating-a-personal-access-token-classic)

The token is a WireTerm named secret. It must be interpolated only into the request header and must not be included in the transform input, Liquid context, logs, fixtures, or rendered HTML.

## Pagination and caps

The reference extension fetches only the first page of each list. It must:

- sort by most recently updated;
- set `shown_count` to the number of returned nodes;
- set `total_count` from `issueCount`; and
- set `truncated` when `pageInfo.hasNextPage` is true.

Do not automatically walk cursors during playlist playback. A single bounded request keeps refresh latency and rate cost predictable, while a frame cannot show hundreds of entries. If a later design needs an exhaustive cache, each alias needs its own cursor and independent pagination; a GraphQL connection page may contain at most 100 items. [GraphQL pagination](https://docs.github.com/en/graphql/guides/using-pagination-in-the-graphql-api)

Labels are separately capped at five per item. The transform records `labels_truncated` when the implementation later requests `pageInfo.hasNextPage` for labels; for the MVP fixture and view, silently showing only the first five labels is acceptable because labels are supplementary display data.

## Template-facing response

The transform must emit this stable shape rather than exposing GitHub's GraphQL envelope directly:

```json
{
  "schema_version": 1,
  "viewer": "sample-user",
  "sections": {
    "opened": {
      "title": "Opened by me",
      "total_count": 1,
      "shown_count": 1,
      "truncated": false,
      "items": []
    },
    "reviews": {
      "title": "Review requests",
      "total_count": 1,
      "shown_count": 1,
      "truncated": false,
      "items": []
    },
    "assigned": {
      "title": "Assigned issues",
      "total_count": 1,
      "shown_count": 1,
      "truncated": false,
      "items": []
    }
  },
  "rate_limit": {
    "cost": 1,
    "remaining": 4999,
    "reset_at": "2030-01-02T03:04:05Z"
  }
}
```

Every `items` entry has:

| Field | Type | Meaning |
| --- | --- | --- |
| `id` | string | GitHub global node ID; stable identity for deduplication |
| `kind` | `"pull_request"` or `"issue"` | Explicit union discriminator |
| `repository` | string | `owner/name` from `repository.nameWithOwner` |
| `number` | integer | Repository-local issue or PR number |
| `title` | string | Unmodified title; Liquid performs display truncation |
| `url` | string | Browser URL |
| `author` | string or null | Login, preserving null for deleted/ghost authors |
| `updated_at` | ISO-8601 string | GitHub update time |
| `draft` | boolean or absent | Present only for pull requests |
| `labels` | array | `{ "name": string, "color": six-hex-digit string }` |

Keep duplicate PRs across `opened` and `reviews` if GitHub ever returns them: sections describe roles, not a globally deduplicated inbox.

## Representative sanitized fixture

```json
{
  "schema_version": 1,
  "viewer": "sample-user",
  "sections": {
    "opened": {
      "title": "Opened by me",
      "total_count": 1,
      "shown_count": 1,
      "truncated": false,
      "items": [
        {
          "id": "PR_example_opened_1",
          "kind": "pull_request",
          "repository": "acme/desktop",
          "number": 42,
          "title": "Add offline playlist playback",
          "url": "https://github.com/acme/desktop/pull/42",
          "author": "sample-user",
          "updated_at": "2030-01-02T02:45:00Z",
          "draft": false,
          "labels": [{ "name": "feature", "color": "1f6feb" }]
        }
      ]
    },
    "reviews": {
      "title": "Review requests",
      "total_count": 1,
      "shown_count": 1,
      "truncated": false,
      "items": [
        {
          "id": "PR_example_review_1",
          "kind": "pull_request",
          "repository": "example-org/service",
          "number": 108,
          "title": "Handle expired credentials",
          "url": "https://github.com/example-org/service/pull/108",
          "author": "review-author",
          "updated_at": "2030-01-02T01:30:00Z",
          "draft": false,
          "labels": [{ "name": "security", "color": "d1242f" }]
        }
      ]
    },
    "assigned": {
      "title": "Assigned issues",
      "total_count": 1,
      "shown_count": 1,
      "truncated": false,
      "items": [
        {
          "id": "I_example_assigned_1",
          "kind": "issue",
          "repository": "sample-user/tools",
          "number": 7,
          "title": "Document the extension manifest",
          "url": "https://github.com/sample-user/tools/issues/7",
          "author": null,
          "updated_at": "2030-01-01T22:00:00Z",
          "labels": [{ "name": "documentation", "color": "0e8a16" }]
        }
      ]
    }
  },
  "rate_limit": {
    "cost": 1,
    "remaining": 4999,
    "reset_at": "2030-01-02T03:04:05Z"
  }
}
```

The fixture intentionally uses fictional identities, repositories, node IDs, URLs, and future timestamps. It contains no token, account-specific value, or captured private response.

## Rate-limit and failure behavior

GraphQL user authentication normally receives 5,000 points per hour. The response headers expose limit, remaining, used, reset time, and the `graphql` resource; the query also requests `rateLimit { cost remaining resetAt }`. [GraphQL rate limits](https://docs.github.com/en/graphql/overview/rate-limits-and-query-limits-for-the-graphql-api#primary-rate-limit)

The extension runner should:

1. poll only when the playlist item is due to refresh, not on every render or preview repaint;
2. avoid concurrent refreshes for the same instance;
3. on exhausted primary limits, retain the last successful data and do not retry before `x-ratelimit-reset`;
4. on a secondary-limit response, honor `retry-after`; otherwise wait at least one minute and exponentially back off;
5. on `502`/`504`, treat the refresh as failed and retry only on the item's next scheduled turn; and
6. surface `rate_limit.remaining` and `reset_at` in diagnostics, not in the default Liquid context.

GitHub recommends webhooks instead of polling where possible, fixed-schedule polling when unavoidable, serial requests, and strict handling of reset/retry headers. Webhooks are inappropriate for this local MVP because WireTerm has no public callback service. [REST API best practices](https://docs.github.com/en/rest/using-the-rest-api/best-practices-for-using-the-rest-api), [GraphQL limit handling](https://docs.github.com/en/graphql/overview/rate-limits-and-query-limits-for-the-graphql-api#exceeding-the-rate-limit)

Unlike authenticated conditional REST `GET` requests, which can return a rate-free `304 Not Modified`, the recommended GraphQL operation is a `POST` and should not assume conditional-cache behavior. Its advantage is one narrow request rather than three broad searches. [REST conditional requests](https://docs.github.com/en/rest/using-the-rest-api/best-practices-for-using-the-rest-api#use-conditional-requests)

## New decisions and remaining fog

- **Resolved here:** use one GraphQL request, three aliased searches, a 20-item default/50-item hard per-section page cap, no automatic pagination, and the normalized schema above.
- **Needs a product decision:** whether a post-MVP authentication flow should use a GitHub OAuth App or GitHub App so users can authorize multiple resource owners without manually creating tokens.
- **Needs implementation validation:** verify the exact GraphQL query against public, private, organization-approved, direct-review, and team-review fixtures when the extension runtime exists.
- **Needs extension-schema ownership:** decide whether the shared extension request model natively supports GraphQL request bodies and aliases or whether the bundled GitHub extension's transform performs the HTTP call itself.
