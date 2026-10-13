# Getting help with PwrRoute

PwrRoute is built by [PwrDrvr LLC](https://pwrdrvr.com). Support is
best-effort and happens on GitHub — there is no ticket queue and no SLA.

PwrRoute is pre-release. There is no build to install yet, so most questions
today are about the design or the port.

## Where to go

| You want to… | Go here |
| --- | --- |
| Report something broken | [New bug report](https://github.com/pwrdrvr/PwrRoute/issues/new?template=bug_report.yml) |
| Request a specific capability | [New feature request](https://github.com/pwrdrvr/PwrRoute/issues/new?template=feature_request.yml) |
| Report a security vulnerability | **Not** in public — follow [SECURITY.md](SECURITY.md) |

## What makes a report answerable

1. **Your version and OS** — Help → About, plus the macOS, Windows or Linux
   version.
2. **Which configuration is in charge** — visual rules, or a Finicky `.js` /
   `.ts` config.
3. **The trace.** Test a link shows each rewrite and handler that fired.
   Replace private hosts and paths before pasting it.
4. **What you expected vs. what happened**, and the exact steps to get there.

## Things that come up a lot

- **URLs are private data.** PwrRoute keeps full URLs in memory only; disk logs
  carry timings, never URL contents. Do not paste URLs into an issue that you
  would not share.
- **Nothing changes the OS default browser on its own.** Setting PwrRoute as
  the default is an explicit action in the app.
- **Finicky configs.** PwrRoute reads Finicky v4 `.js` and `.ts` configs. It
  is not Finicky and is not supported by Finicky's authors; report PwrRoute
  problems here, not upstream.

## Response expectations

Small team, public queue. Security reports are triaged as a priority through
the private channel in [SECURITY.md](SECURITY.md).

Contributing code instead? See [CONTRIBUTING.md](CONTRIBUTING.md).
