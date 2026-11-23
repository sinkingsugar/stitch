---
name: edge-talk
description: The user's edge talk history (the chat app they used before Stitch): search and read its chats and the memories it distilled, with edge_talk_search, edge_talk_memories, edge_talk_conversations and edge_talk_read. Use it when the user mentions edge talk or chats from before Stitch, or when recall finds nothing about older history.
---

## Edge talk history

Edge talk is the chat app the user used before Stitch (its successor).
Its history stays where it is, in edge talk's own database on this
device; these tools read it, never change it, and run without asking.
They are lazy: not in your tool list, but called with `Tool.Call` in
`otter_run` (the otter skill), their parameters given below when you read
this with `read_skill`; a search and the reads of its hits can go in one
run:

```otter
{query: "kelp forests" limit: 5} | Tool.Call("edge_talk_search")
```

- `edge_talk_search {query, from?, to?, limit?}`: full-text search over
  messages, best first: each hit's conversation id and title, date,
  sender and a snippet. Words match by stem and prefix (`tile` finds
  `tiles`); all must match, else any of them. Short messages (a user's
  under 50 characters, an assistant's under 200) and tool rows are not
  indexed.
- `edge_talk_memories {query?, from?, to?, limit?}`: the memories edge talk
  distilled from the chats (facts, preferences, events about the user),
  by search, or the latest without a query.
- `edge_talk_conversations {title?, from?, to?, limit?, offset?}`:
  conversations, the latest first, by words in the title and the days
  they were active.
- `edge_talk_read {id, offset?, limit?, max-chars?, include-tools?}`: one
  conversation in order, a page at a time, long messages cut.

Dates are `YYYY-MM-DD` (`to` included).

When to use them:

- The user mentions edge talk, or something said or done before Stitch
  ("we discussed this last year", "what did I decide about…").
- `recall` (Stitch's own memory) finds nothing about older history: edge
  talk's goes back further.

How to use them well:

- Start with `edge_talk_search` (or `edge_talk_memories` for facts about
  the user), then `edge_talk_read` the conversations that look right; page
  with `offset` rather than raising `limit` far.
- Search with a few distinctive words, not a sentence; try synonyms when
  nothing comes back, and narrow with `from` and `to` when too much does.
- Say that what you found is from edge talk, with its date: it may be out
  of date.
- It is the user's private history: quote what answers the question, not
  whole conversations.
