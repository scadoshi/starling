# Azure Setup

Starling signs in as an Azure app that you register, so the only permissions it has are the ones you grant. It needs a work or school account: Microsoft's chat API doesn't serve personal accounts.

If your organization doesn't let you register apps, ask your IT admin to do these steps and send you the client ID.

## Register the app

1. Open the [Microsoft Entra admin center](https://entra.microsoft.com/) and go to App registrations, then New registration.
2. Name it anything, such as `Starling`. For supported account types, pick "Accounts in any organizational directory": Starling signs in through Microsoft's common endpoint, which single-tenant apps can't use. Leave the redirect URI empty; device-code sign-in doesn't use one.
3. Register, and copy the Application (client) ID from the overview page. It looks like `12345678-1234-1234-1234-123456789abc`.

## Allow device-code sign-in

Under Authentication, find "Allow public client flows" in the advanced settings, set it to Yes, and save.

## Grant permissions

Under API permissions, add a permission, choose Microsoft Graph, then Delegated permissions, and add:

- `User.Read`, to read your name
- `Chat.ReadWrite`, to read and send chat messages
- `offline_access`, to stay signed in between runs

Delegated, not Application: application permissions would let the app read every chat in the organization, and Starling never asks for that. If your organization requires admin consent, an admin has to grant it here before sign-in works.

## Sign in

Run `starling`, paste the client ID, and follow the code it shows.

## When sign-in fails

Starling shows Microsoft's error on the client ID screen. The common ones:

| Error | Cause |
| --- | --- |
| `AADSTS50059` | No app has that client ID. Check you copied the Application (client) ID, not the object or tenant ID. |
| `AADSTS7000218` | Public client flows are off. |
| `AADSTS65001` or "admin consent required" | Your organization requires an admin to approve the permissions. |
| A scope "does not exist" | The permissions above are missing or were added as Application permissions. |

After changing permissions, delete `token.json` from the config folder (see the README) so the next sign-in picks them up.
