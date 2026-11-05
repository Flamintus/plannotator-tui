# plannotator-tui-inbox

A client of the Plannotator Inbox's person API, for terminal clients.
The Inbox is Plannotator's local server for threads between a person and coding agents;
its device door, `/api/inbox/device/*`, is the API of every client that is not its own window.
This crate speaks it synchronously, with no runtime: plain HTTP on loopback (`ureq`, no TLS).

It is written against contract E7, before E7 exists on the Inbox's side.
Routes E7 has not put on the door yet answer `Error::RouteMissing`;
local pairing on an Inbox without E7 answers `Error::RouteMissing` (Phones off)
or `Error::Refused` with `device_token_invalid` (Phones on), never `Error::Unpaired`.

## Shape

- `discover(data_dir)` reads `inbox/inbox.json` under Plannotator's data directory
  and checks the Inbox's health on the loopback port it names.
  The data directory is the caller's: `plannotator_tui_schema::data_dir`,
  which honours `PLANNOTATOR_DATA_DIR`.
- `connect(data_dir, name)` also pairs this terminal, or reuses the token it kept,
  and replaces a token the Inbox revoked.
- `Inbox` is the seam: `Client` speaks HTTP; an application's tests put a stand-in,
  and hand it a `Watch::from_events` channel for the event stream.
- `Inbox::watch` follows the event stream on threads of its own:
  it resumes by `Last-Event-ID`, keeps one thread's records, gathers a burst into one event,
  says when the Inbox restarted, skips kinds it does not know,
  and stops on a contract newer than `CONTRACT`.
  The caller reads the computed view again; nothing is replayed into state here.
- The registry is read again for every call, as the Inbox asks of its callers:
  a restart may move the port.
  The registry stays in place when the Inbox stops,
  so the health answer on its port must name its `serverSession`
  before any request carries the device token.
  This keeps the token from a process that took the port later,
  unless that process learnt the `serverSession` while the Inbox ran:
  the health route and the event stream show it to any local caller.
- `send` takes the press's `IdempotencyKey`:
  the same press tried again after an error goes under the same key and never sends twice.
  The other writes are idempotent by the comment's id, and mint their own key.

## Routes

Door routes, each served by the window route of the same name.

| Call | Route | On the door today |
|---|---|---|
| `health`, `sessions`, `threads`, `thread` | `GET health`, `sessions`, `threads`, `threads/:id` | yes |
| `attachment_text` | `GET attachments/:id/view?version=` | yes |
| `comment_on_message` | `POST messages/:id/annotations` | yes |
| `comment_on_attachment` | `POST annotations` | yes |
| `remove` | `POST annotations/:id/remove` | yes |
| `watch` | `GET events` | yes, without `v` and `thread_id` on every line |
| `thread_view` | `GET threads/:id/comments?v=1` | E7 |
| `reply`, `resolve` | `POST annotations/:id/replies`, `annotations/:id/resolve` | E7 (window only) |
| `sub_thread` | `POST annotations/:parent/subthreads` | E7, by passage |
| `send` | `POST threads/:id/comment-sends` | E2 and E7 |
| `deposit` | `POST review-targets` | P1 and E7 |
| `pair_local` | `POST pair-local` | E7 |

Shapes this client assumes where E7 is not written yet, to settle with E7:

- `pair-local` takes the registry's connection token as its bearer
  and `{ name, platform: "terminal", idempotency_key }`, and answers `{ token, device }`.
  The device token is kept in `clients/plannotator-tui/inbox-device.json`,
  0600 in a 0700 folder on Unix; on Windows the folder's own access rules apply.
- A sub-thread is posted as `{ annotation: { id, text, quote, occurrence } }`,
  `occurrence` counting from 0 the earlier occurrences of `quote` in the parent's words;
  the server computes `quote_offset`.
- `comment-sends` takes `{ mode, comment_ids? }` and answers `{ sends: [...] }`.
- `review-targets` takes `{ session: "<host>:<id>", project_path?, target }`,
  `target` being `{ kind: "chat_message", text, host_message_id }` or `{ kind: "file", path }`,
  and answers `{ thread_id, message_id, attachment_id?, created }`.
- The thread view is E1's `InboxCommentThreadView` as it is: `{ v, round, threads }`.

## Tests and fixtures

The tests run against a stand-in Inbox on loopback (`tests/inbox/support.rs`)
in a fresh data directory under the temp dir: never a real Inbox, never `~/.plannotator`.
The stand-in answers from the contract fixtures, `tests/fixtures/v1/`, one per answer read.

- `thread-view.json` is computed by Plannotator's own code for the view
  (E1's `inboxCommentThreadView`, commit `0b6a9d4d0`),
  through `thread-view.gen.ts` (`PLANNOTATOR_REPO=<checkout> bun run …`).
- The others are written by hand from the Inbox's code:
  `events.sse` from `eventStream` and `eventPayload` (`packages/server/inbox.ts`), as before E7;
  `health`, `sessions`, `threads`, `thread`, `attachment-view` and `annotation`
  from the window routes and their types;
  `comment-sends`, `review-target` and `pair-local` from the shapes assumed above.

Once E7 exists, all of them are regenerated by a Bun test on Plannotator's side,
against a sandboxed Inbox, so that a change of the contract breaks both sides.
