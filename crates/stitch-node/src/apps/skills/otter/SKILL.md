---
name: otter
description: Write, check and run code in Otter, the language Stitch's tools are written in, with otter_search, otter_describe, otter_check and otter_run, on any node (the phone too); and write a skill's own tools in it.
---

## Otter

Otter is a data-flow language: a program is a pipeline of shards, each
taking the previous one's output as its input. A whole program is checked
before it runs (types, parameters, variables), so most mistakes are compile
errors with a line and column, not surprises later. It succeeds Shards 1.x
(edge talk's scripts): 1.x code and shard names often do not exist here.
Scripts are `.otr` files (`.otter` and `.shs`, the old extension, work too).

Four tools come with this skill, on every node; on the phone, which has no
shell, they are the only way to run Otter. They answer as the `otter`
command does (`otter search`, `describe`, `check`, `run`):

- `otter_search {query}`: shards by name or summary, best first, each as
  `Name (input -> output): summary`.
- `otter_describe {name}`: one shard in full (JSON): its input and output,
  every parameter with its types, help and default, its effects.
- `otter_check {code}` (or `{file}`): problems with `line:column`, or
  `code: ok` and the effects a run would have.
- `otter_run {code}` (or `{file}`, and `input`, a JSON value for the
  top-level code): what it logged, then `root: <value>`, or why it failed.

### The loop: discover, check, run

1. **Never guess a shard.** Find it with `otter_search` and read it with
   `otter_describe` before writing a call: names, parameter labels and
   whether a value is the input or a parameter are where guesses go wrong
   (`Http.Get` takes its URL as `url:`, not as input).
2. **Check before running.** `otter_check` until it says `ok`; each
   diagnostic points at `code:line:column` and often says the fix.
3. **Run.** Code that only computes runs at once. Code with effects (the
   network, files, the clock, waiting, randomness) asks the user first, and
   is refused in a read-only thread, so check it first and keep such runs
   few. `Log` counts as I/O: to see a value without asking, make it the
   code's last value, which the run prints as `root: <value>`.

### The language in brief

```otter
// `|` passes each shard's output to the next one; it is optional.
7 | Math.Add(5) | Math.Multiply(3)
// → root: 36
```

- Values: `Int` (`42`), `Float` (`1.5`; write `2.0`, never `2.`), `Bool`,
  `String` (`"text"`), `none`, sequences (`[1 2 3]`), tables
  (`{name: "Ann" age: 41}`).
- `value = x` names a value (immutable); `value | Var(x)` declares a
  mutable variable, `value | Update(x)` assigns it, `value | Push(xs)`
  appends to a sequence variable. Names are lowercase and may hold `-`, so
  `a-1` is one name: subtract with `Math.Subtract`.
- `x.key`, `xs.0`, `x.a.b.1` read a table key or a sequence element
  (`Take`); a bare path is a step on the pipeline value: `x | .a.b`,
  `Map({.name})`. On values of unknown shape (parsed JSON) a path is
  checked at run time; a `?` at its end makes it lenient: `j.data.3.0?`
  is `none` when any step cannot apply (not a table, no such key, too
  short) instead of failing.
- `= {a b}` binds the fields `a` and `b` at once, `= {a: x b: y}` binds
  them as `x` and `y`; `= [a _ c]` binds a sequence's first elements by
  position, `_` skipping one.
- `f"{name} has {xs | Count} items"` builds a string from pipelines.
- Shards start with an uppercase letter; parameters go positionally or by
  lowercase label: `Repeat({...} times: 3)`. A `{...}` block is a flow the
  shard runs.

```otter
{name: "Ann" langs: ["Otter" "Rust"]} = person
person.langs | Count = n
f"{person.name} writes {n} languages: {person.langs | String.Join(", ")}"
// → root: "Ann writes 2 languages: Otter, Rust"
```

Loops over a collection: `ForEach`, `Map`, `Filter`, `Reduce`. Names
before the flow are the element, then its index (a table's value, then
its key); `Reduce` names the accumulator first. Names are optional, `_`
skips one, and the body's input is the element.

```otter
[{name: "Ann" age: 41} {name: "Bo" age: 9} {name: "Cy" age: 30}] = people
people | Filter({.age | IsMore(17)}) | Map({.name}) = adults
people | Reduce(total p {p.age | Math.Add(total)} init: 0) = years
people.0 = {name age: first-age}
f"{adults | String.Join(", ")}; {years} years; {name} is {first-age}"
// → root: "Ann, Cy; 80 years; Ann is 41"
```

- `seq | ForEach(x i {...})` passes its input through; `Map` outputs the
  body's outputs (a table gives a table with the same keys); `Filter`
  keeps what its predicate is `true` for; `Reduce(acc x {...} init: v)`
  outputs the last accumulator (without `init:` it starts from the first
  element).
- No `Break`: guard the body with `When`, or use `Repeat(... until:)` or
  `Return` from a function to stop early. A `String` is not iterable:
  `String.Split` first.

Other control flow: `If({predicate} {then} {else})`, `When`, `While`,
`Repeat({...} times: n)` or `until:`, `Match([value {flow} ...] default:
{...})`, `Maybe({try} {fallback} silent: true)`, `Fail` (fails with its
input as the message), `Stop`, `Return`.

```otter
[3 8 1] = xs
0 | Var(total)
xs | ForEach(x {x | Math.Add(total) | Update(total)})
total | If({IsMore(10)} {"big"} {"small"})
// → root: "big"
```

Functions are declared at the top level with their types and called like
shards, parameters by label. A function sees only its input (`input`), its
parameters and what it declares.

```otter
@fn(Scale input: Float output: Float params: {factor: Float} {
  Math.Multiply(factor)
})
[1.5 2.0] = xs
xs | Take(1) | Scale(factor: 3.0)
// → root: 6.0
```

Values of unknown shape (parsed JSON, a variable key) must be narrowed
before use where a type matters: `ExpectString`, `ExpectInt`,
`ExpectTable`, `ExpectIntSeq` and the rest fail on another type; with
`default:` (a value of the expected type) they give the default instead,
none included, so an optional argument or a JSON field that may be
missing is one shard: `args.limit | ExpectInt(default: 30)`,
`v.runs? | ExpectSeq(default: [])`. No `IntOr`/`StringOr` helpers to
write. Paths into them need no narrowing (checked at run time; `?` for
none instead of a failure).

```otter
"{\"items\": [1, 2, 3]}" | FromJson | ExpectTable = doc
doc.items | ExpectIntSeq | Count = n
doc.missing.0? = nothing
doc.limit? | ExpectInt(default: 30) = limit
[n nothing limit]
// → root: [3 none 30]
```

`First` and `Last` give a sequence's ends; an empty one fails unless
`default:` is given (`Last(default: none)`). `String.PadStart` and
`String.PadEnd` pad to a `width` in characters (`with:` one character,
`" "` by default).

```otter
[4 7 9] = xs
[(xs | First) (xs | Last) ([] | Last(default: 0))
 (7 | ToString | String.PadStart(width: 2 with: "0"))]
// → root: [4 9 0 "07"]
```

```otter
Maybe({"not a number" | ParseInt} {-1} silent: true)
// → root: -1
```

Effects come from the shards a program reaches: `otter_check` lists them.

```otter
Http.Get(url: "https://example.com") = page
page | Count
// effects: io, time, suspends
```

### The thread's tools from code: `Tool.Call`

In `otter_run`, `Tool.Call` calls any tool this thread has, by name, as if
you had called it: its input is the arguments (a table as the tool's schema
says; `{}` or none for no arguments), its output the tool's text result, a
String (`FromJson` reads a JSON one). Use it to call a skill's lazy tools
(not in your tool list: the skill's line says which, and `read_skill`
gives their parameters), and to combine calls in one run: five videos,
five dates, a search then a read of each hit.

```otter
{video: "dQw4w9WgXcQ" max_chars: 4000} | Tool.Call("youtube_video")
// effects: io, time, suspends; all of them Tool.Call's
```

- **Approvals stay per tool.** `otter_run` does not ask because of
  `Tool.Call`; each call is approved as that tool is when called directly
  (a tool that asks asks, the question saying it comes from otter_run; one
  allowed runs unasked; a read-only thread refuses what it refuses). The
  rest of the code asks for its own effects as usual.
- A failed call fails the shard with the tool's message: `Maybe` catches
  it (Maybe may log, so a run with it asks once for itself).
- Images a tool returns are left out, with a note: call that tool directly
  to see them.
- Not callable: `otter_run`, `spawn`, `send`, `wait`, `cancel`, `wake_at`,
  `schedule`. Only in `otter_run`: a skill's tool script has no tools to
  call.
- A Tool.Call takes the previous output as its arguments: start its line
  with the table (`{} | Tool.Call("x")`), not after a `Log`.

```otter
// Two dates in one run; their results together.
["2026-11-02" "2026-11-09"] = dates
dates | Map(date {
  {from: "KIX" to: "NRT" date: date} | Tool.Call("google_flights_search") = found
  f"## {date}\n{found}\n"
}) | String.Join
// effects: io, time, suspends; all of them Tool.Call's
```

### A skill's own tools

A skill brings tools as scripts in its `tools/` folder, one tool a file:
`<home>/skills/<skill>/tools/<tool>.otr`, or in a place
`.stitch/skills/<skill>/tools/<tool>.otr` (next to that skill's
`SKILL.md`, which says when to use them). They are offered while the skill
is on, from the next turn, after the built-in tools, on every node that
has the skill's files. A file with no top-level code is a library the
others `@include("lib.otr")`; it is not a tool.

A tool script declares its function and ends with its definition, the
table its top-level code outputs: `definition` is what the model sees
(`parameters` is JSON Schema), `use` names the function a call runs.

```otter
// tools/word-count.otr: it only computes, so it never asks.
@fn(WordCount input: {input: {} env: {} working-dir: String} output: String params: {} {
  input.input.text | If({IsString} {ExpectString} {"text is required" | Fail}) = text
  text | String.Split(" ") | Count = words
  f"{words} words"
})

{
  definition: {
    name: "word_count"
    description: "Counts the words in a text."
    parameters: {type: "object" properties: {text: {type: "string"}} required: ["text"]}
  }
  use: "WordCount"
}
```

- The function takes `{input env working-dir}`: `input` the model's
  arguments (check them: they are whatever the model sent; an optional one
  reads as `input.input.limit | ExpectInt(default: 30)`), `env` an empty
  table for now, `working-dir` the thread's directory. It outputs the
  result: a String as it is, any other value as JSON. `Fail` with a
  message makes the call fail with it.
- The top-level code only builds the definition: no I/O, waiting or clock.
- Names are `[A-Za-z0-9_-]`, at most 64 characters, without `__`, and not a
  built-in tool's.
- A tool with no effects runs without asking, in read-only threads too.
  Any effect asks the user, shown as `run [otter:<skill>]: <tool>
  <arguments>`, unless `[otter] allow` or an "always" answer allowed it.
- Files are confined to the thread's directory (read-only in a read-only
  thread): build paths from `working-dir`.
- `tools: lazy` in the skill's front matter (next to `name` and
  `description`) keeps its tools out of the tool list: they are called with
  `Tool.Call` in `otter_run`, and `read_skill` gives their parameters. For
  tools needed in few threads; without the key they are listed.

```otter
// tools/save-note.otr: it writes a file, so it asks.
@fn(SaveNote input: {input: {} env: {} working-dir: String} output: String params: {} {
  input.input.note | If({IsString} {ExpectString} {"note is required" | Fail}) = note
  [input.working-dir "notes.txt"] | FS.Join | FS.Write(contents: note) = path
  f"saved in {path}"
})

{
  definition: {
    name: "save_note"
    description: "Saves a note in notes.txt, in the directory the thread works in."
    parameters: {type: "object" properties: {note: {type: "string"}} required: ["note"]}
  }
  use: "SaveNote"
}
```

Write the file (`write_file`), check it with `otter_check {file:
".stitch/skills/<skill>/tools/<tool>.otr"}`, and fix it until it says
`ok` (a home workspace skill's file is outside the thread's directory:
check its text as `code`); a script that does not compile is not offered,
and the node's log says why. A real example, two tools over a shared library: the
`google-flights` skill's `tools/flights.otr`, `tools/search-flights.otr`
and `tools/cheap-dates.otr` (`read_skill` with `name: "google-flights"`
and `file`).
