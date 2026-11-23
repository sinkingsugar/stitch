---
name: youtube
description: Watch a YouTube video by reading it: its details, timestamped transcript (in the language asked for, or the video's own) and top comments, with the youtube_video tool. Use it when the user shares a YouTube link or asks about a video.
---

## YouTube

One tool comes with this skill, written in Otter and run by the node; it
asks YouTube's own API as the Android app and the website do (no key). It
is lazy: not in your tool list, but called with `Tool.Call` in `otter_run`
(the otter skill), its parameters given below when you read this with
`read_skill`. The result is the tool's text, as the run's value:

```otter
{video: "https://youtu.be/dQw4w9WgXcQ"} | Tool.Call("youtube_video")
```

Several videos in one run: one `Tool.Call` each, their results joined
(`f"{a}\n\n{b}"`). Each call asks as the tool does when called directly
(it asks, unless `[otter] allow` or an "always" answer allows it); the run
itself does not ask for them.

- `youtube_video {video, language?, omit_captions?, omit_comments?,
  from_seconds?, max_chars?, max_comments?}`: the title, channel, length,
  views and description; the transcript, one timestamped line per caption
  (`[12:34] …`); and the top comments with their likes.

How to use it well:

- `video` takes the ID or any YouTube URL as the user gave it (`watch?v=`,
  `youtu.be/`, `shorts/`, `embed/`, `live/`; a `&t=` is ignored).
- `language` is the transcript's (`en` by default): use the user's, or
  the video's when they want the original. Captions written by people are
  taken before automatic ones; without any in that language, the
  transcript is in the video's own and the result says so: translate it
  yourself (YouTube's translations refuse requests without a session). Automatic captions
  have no punctuation and mishear names: say so when it matters.
- Long transcripts are cut at `max_chars` (60,000 by default, about an
  hour of speech); the result says the second it stopped at, and
  `from_seconds` continues from there. For one part of a long video, start
  at its time.
- For a summary or questions about the content, read the transcript;
  cite times (`[12:34]`) so the user can jump there. Only the details and
  comments: `omit_captions: true`; only the transcript:
  `omit_comments: true`.
- A video that is private, removed or age-restricted gives no details:
  say so rather than guessing its content from the title.

Its scripts are in `tools/` (`youtube.otr` the reading of YouTube's JSON,
`video.otr` the requests and the tool); they are a port of edge talk's
youtube-captions tool.
