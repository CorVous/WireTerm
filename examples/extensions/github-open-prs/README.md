# GitHub open pull requests

This ready-to-copy WireTerm 0.1.0 Extension shows up to five recently updated
open pull requests authored by a GitHub username. It uses GitHub's official
REST Search API, a WireTerm named-secret header binding, and direct black,
white, and red SVG only.

## Install

1. Copy this whole `github-open-prs` folder into
   `wireterm-data/extensions/github-open-prs` beside `wireterm.exe`.
2. Create a GitHub personal access token with an expiration. Prefer a
   fine-grained token restricted to the resource owner and repositories that
   should appear. Grant only read-only **Pull requests** access when private
   repositories are required. Public-only results do not need additional
   repository permissions, but this sample still requires a token for a
   consistent authenticated rate limit. GitHub documents token creation and
   permission selection in [Managing your personal access tokens](https://docs.github.com/authentication/keeping-your-account-and-data-secure/managing-your-personal-access-tokens).
3. In WireTerm, expand **Advanced details**, create a named secret such as
   `github-read`, and set its value to `Bearer YOUR_TOKEN`. The `Bearer ` prefix
   is required because WireTerm injects the stored value as the complete
   `Authorization` header. MVP named secrets are stored unencrypted in the
   adjacent `wireterm-data` folder; protect access to that folder.
4. Choose **+ Extension**, select **GitHub open pull requests**, bind its
   **GitHub token** input to `github-read`, and leave **GitHub username** as
   `CorVous` or replace it with another account name.
5. Apply the item, select it, and use **Refresh preview**. Preview performs one
   bounded read-only request and does not send a panel frame or consume a
   playback turn.

Never paste a token into the username setting or into `extension.lua`. The
script passes only the opaque `github_token` reference through
`secret_headers`; the secret value is injected by WireTerm at the final HTTP
request boundary.

## GitHub access caveats

The search includes only repositories visible to the token. Fine-grained
personal access tokens are tied to one resource owner, so private pull requests
spread across multiple organizations may require separate token/Extension
instances. A classic token with the `repo` scope can span private repositories
the account can access, but it is substantially broader and may require
organization approval or SSO authorization. Prefer the narrow fine-grained
option whenever it covers the required repositories.

The Extension requests five results sorted by `updated` descending. GitHub may
return partial search results while indexing; the header says so. Authentication,
permission, rate-limit, malformed-response, and invalid-username failures are
reported without including the token, request URL, or response body. Empty
results render a normal "No open pull requests" screen.
