// Writes thread-view.json with Plannotator's own code for the thread view (brick E1,
// `inboxCommentThreadView` in packages/core/inbox-comment-sends.ts), so the Rust types are
// checked against what the Inbox computes. Pure: it reads no store and writes nothing.
//
//   PLANNOTATOR_REPO=<a Plannotator checkout with E1> bun run thread-view.gen.ts > thread-view.json
//
// To be replaced by a Bun test on Plannotator's side once E7 serves the view on the door.
const repo = process.env.PLANNOTATOR_REPO;
if (!repo) throw new Error("PLANNOTATOR_REPO: a Plannotator checkout with packages/core/inbox-comment-sends.ts");
const { inboxCommentThreadView } = await import(`${repo}/packages/core/inbox-comment-sends.ts`);

const at = (minute: number) => `2026-11-05T10:${String(minute).padStart(2, "0")}:00.000Z`;
const agent = (id: string, minute: number, session = "ses_a") => ({ id, author: { kind: "agent", session }, created_at: at(minute) });
const person = (id: string, minute: number) => ({ id, author: { kind: "person" }, created_at: at(minute) });

function record(id: string, fields: Record<string, unknown> = {}) {
  return {
    id, project_id: "prj_1", thread_id: "msg_a1", message_id: "msg_a1", attachment_id: "", path: "msg_a1", version: "current",
    message: { at: at(0) },
    annotation: { id, type: "COMMENT", originalText: `words of ${id}`, text: `comment ${id}` },
    created_at: at(1), updated_at: at(1), removed_at: null, sent_reply_id: null,
    ...fields,
  };
}
function send(id: string, minute: number, fields: Record<string, unknown> = {}) {
  return { id, project_id: "prj_1", thread_id: "msg_a1", batch: id, to: { host: "claude-code", session: "ses_a" }, round: 1, mode: "now", comments: [], with_message: null, idempotency_key: null, created_at: at(minute), ...fields };
}
function agentReply(id: string, re: string, minute: number, name: string | null = "Claude Code") {
  return { id, project_id: "prj_1", thread_id: "msg_a1", re, from: { kind: "agent", host: "claude-code", session: "ses_a", name }, message_id: "msg_a2", text: `answer ${id}`, created_at: at(minute) };
}

const messages = [person("msg_p0", 0), agent("msg_a1", 0), agent("msg_a2", 4)];
// Carried by the person's reply, before comment sends: no status.
const legacy = record("c0", { sent_reply_id: "msg_p0", created_at: at(0) });
// A comment on the message, sent now, answered, live.
const c1 = record("c1", { sent_reply_id: "snd_1", created_at: at(1) });
const rep = agentReply("rep_a", "c1", 4);
const r1 = record("r1", { re: "c1", sent_reply_id: "snd_2", created_at: at(6), annotation: { id: "r1", type: "COMMENT", originalText: "words of c1", text: "and?" } });
// Sub-threads: on the agent's reply, on the comment, on a sub-thread.
const s1 = record("s1", { parent: "rep_a", quote_offset: 7, sent_reply_id: "snd_2", created_at: at(7), annotation: { id: "s1", type: "COMMENT", originalText: "rep_a", text: "this" } });
const s2 = record("s2", { parent: "c1", quote_offset: 0, sent_reply_id: "snd_2", created_at: at(8), annotation: { id: "s2", type: "COMMENT", originalText: "comment", text: "that" } });
const s3 = record("s3", { parent: "s1", quote_offset: 0, sent_reply_id: "snd_2", created_at: at(9), annotation: { id: "s3", type: "COMMENT", originalText: "this", text: "deeper" } });
// A code comment on a file's sent version, unanchored since.
const f1 = record("f1", {
  message: undefined, attachment_id: "att_01JC0000000000000000000000", path: "/repo/src/main.rs", version: "a".repeat(64),
  annotation: { id: "f1", type: "COMMENT", originalText: "fn main()", text: "rename", inboxCode: { lineStart: 3, lineEnd: 5 } },
  sent_reply_id: "snd_2", created_at: at(8), unanchored_at: at(11),
});
// A comment on a video's moment, current version.
const v1 = record("v1", {
  message: undefined, attachment_id: "att_01JC0000000000000000000001", path: "/repo/demo.mp4", version: "current",
  annotation: { id: "v1", type: "COMMENT", originalText: "", text: "too fast", blockId: "video", inboxVideo: { time: 42.5 } },
  sent_reply_id: "snd_2", created_at: at(8),
});
// A guided review's line comment, sent.
const g1 = record("g1", {
  message: undefined, guide: { title: "Review", file: "src/lib.rs", side: "new", line_start: 10, line_end: 12 }, version: "b".repeat(64),
  annotation: { id: "g1", type: "COMMENT", originalCode: "let x = 1;", text: "why?" },
  sent_reply_id: "snd_2", created_at: at(8),
});
// Resolved, nothing open under it: folded.
const done = record("c9", { sent_reply_id: "snd_1", created_at: at(1), resolved_at: at(5) });
// The person's drafts: a reply under c1, a new comment.
const d1 = record("d1", { re: "c1", created_at: at(12), annotation: { id: "d1", type: "COMMENT", originalText: "words of c1", text: "one more thing" } });
const d2 = record("d2", { created_at: at(13) });
const sends = [
  send("snd_1", 2, { comments: ["c1", "c9"], delivery: { state: "delivered", host: "claude-code", session: "ses_a", at: at(3) } }),
  send("snd_2", 10, { mode: "round", comments: ["r1", "s1", "s2", "s3", "f1", "v1", "g1", "c1"] }),
];

const view = inboxCommentThreadView({
  messages,
  records: [legacy, c1, r1, s1, s2, s3, f1, v1, g1, done, d1, d2],
  agentReplies: [rep],
  sends,
  busy: () => false,
});
console.log(JSON.stringify(view, null, 2));
