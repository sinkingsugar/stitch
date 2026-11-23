# Studio: an app with threads of its own

A small demo of app threads (design §2.22): project cards, a new project
(a thread of the app's own, from a brief and an optional sketch), and a
conversation with that project's agent beside a turning mannequin.

- `stitch-app.json` declares what its threads need: a folder of each
  thread's own (`global`), the `files`, `viewer` and `images` tools, the
  default and quick models, $1 a thread, $5 a day, two at work at once.
- `app.js` uses `stitch.threads` (`create`, `list`, `messages`, `send`,
  `cancel`, `on`) and keeps which thread is which project in
  `stitch.storage`.

To try it, ask an agent in a conversation to show it:

> Show the app in `examples/app-threads` as "Studio".

Then grant it what it asks (the page shows the command; without
`--needs` it only shows what it asks):

```sh
stitch app allow-threads "Studio" --thread <that conversation> --needs <hash>
```

Against a test node with a mock model instead of a real one:
`tools/app-threads/try.sh` (a node of its own in a temporary data
directory, on port 7277; nothing of yours is touched).
