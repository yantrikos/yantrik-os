# Designing a describe a mind can use

A mind does not see your window. What it knows of your program is what `describe` says, and what it
can do is what the actions say they do. All of it goes into a model's context beside every other
program on the machine, so write it the way you would write a note to a capable colleague who
cannot see the screen: short, specific, in your program's own words, and honest about what each
thing costs.

A mind reads in three steps, and each step reads less of you than the last:

1. **Which program?** `yos ls` and the shell's app list give one line per program: the summary of
   each one that is running, and the `X-Yantrik-Purpose` of each one that is closed
   ([Being found while closed](found-while-closed.md)).
2. **What is it holding?** `describe` on the one it chose: summary, state, actions.
3. **Do this.** An act, whose answer carries the result and the view after it — which is often all
   the mind reads before its next step.

## The summary

One line a person could read. Start with the program, then what distinguishes this moment: what is
open, how many, what is unsaved, what is in progress. It is the one part a mind reads for every
program on the machine at once.

```text
Counter — 2, last changed by pid 1735326
Hello — 2 items, clearing
Notes — “Kernel asks”, 412 words, unsaved
LibreOffice — report.odt (Writer, unsaved changes); budget.ods (Calc); budget.ods in front
```

LibreOffice's is built from the same read as its state, and counts instead of listing when there
are many documents:

<!-- from: adapters/libreoffice/yantrik_libreoffice/surface.py -->
```python
def summary_of(documents, front):
    """One line a person could read."""
    if not documents:
        return "LibreOffice — running, no document open"
    if len(documents) > 3:
        return "LibreOffice — %d documents open%s" % (
            len(documents), "; %s in front" % front if front else "")
```

Not `OK`, not `Ready`, not a JSON dump, and not the same line whatever is happening.

## The state

An object in your program's own vocabulary, holding what a mind would act on — above all the
**names your actions take**. If an action takes `document`, the state lists documents by the name
that works as `document`; if an action takes `index`, the state lists items with their `index`.
A mind that has to guess an identifier guesses wrong.

<!-- from: adapters/libreoffice/yantrik_libreoffice/office.py -->
```python
    def _summary(self, doc):
        kind = self.kind(doc)
        out = {"name": self.name_of(doc), "kind": kind, "path": self.path_of(doc),
               "modified": bool(doc.isModified()), "read_only": bool(doc.isReadonly())}
```

Keep it small. It is read around every act and handed to a model, so a document's text or a
sheet's cells do not belong in it: publish their size and an action to read them (`read_text`,
`read_cells`, with a range and a page size).

**Everything in the summary and the state is the revision.** The revision is a hash of both, and a
mind that sends it back as `expect_revision` is refused with `STALE:` if anything in them changed
— so put in what a stale decision would be wrong about, and leave out what changes without
mattering. A clock, a timer or a rate in the state makes every read stale; an unsaved-changes flag
alone misses a second edit to a document that was already unsaved. LibreOffice publishes a
fingerprint of each document's content for that reason:

<!-- from: adapters/libreoffice/yantrik_libreoffice/office.py -->
```python
        if kind == WRITER:
            text = doc.getText().getString()
            out["characters"] = len(text)
            out["content"] = "%08x" % zlib.crc32(text.encode("utf-8"))
```

Keep very small and very large floats out of it (their JSON renderings are where implementations
disagree about the revision), and never put a secret in it: the state is shown, logged and handed
to models.

**In Private mode none of it is shown.** While the person has Private mode on, every surface
answers `describe` with the private view: your id, `"private": true`, the line
`Private mode is on; this app's contents are hidden.` as the summary, and of your state only the
keys you listed as not personal — `table.shown_in_private(&["units"])` in an app's `intents`, or
`surface.show_in_private("units")` in Python. The default is none. List a key only when its whole
value says nothing about the person (a unit, a CPU figure), never a title, a name, a path, a place
or a count of their things. The rule is in
[surface-protocol.md](../surface-protocol.md#in-private-mode).

## Actions

**Names** are a verb and what it acts on — `add_event`, `export_pdf`, `read_cells` — and there is
one action per thing a mind can do. Two actions that do the same thing, or one action that does
two things depending on a flag, are where a mind picks wrong.

**Descriptions** say what the action does, in the program's words; what it does *not* do, when a
mind could assume it; what the answer holds; and what cannot be taken back. The description is
also the sentence on the approval card the person reads. Email's `compose` is the model of the
second part:

<!-- from: apps/email/src/intents.rs -->
```rust
            Action::new(
                "compose",
                "Open the composer with a draft filled in. Does NOT send — the user reviews and sends it.",
            )
```

and LibreOffice's `open` of the other three — it names what is left alone, what it will not do
twice, and what it answers with:

<!-- from: adapters/libreoffice/yantrik_libreoffice/surface.py -->
```python
        """Open a file in LibreOffice, in a window of its own; documents already open stay as
        they are, and a file already open is not opened twice. Answers with the name the other
        actions take as `document`."""
```

**Parameters** carry their type, and the dispatch enforces it. Use the narrowest type that fits:
an integer for an index, an enum (`Param::one_of`, or `Literal[...]` in Python) for a fixed set of
choices, a default wherever there is an obvious one. Describe each one with its format and where
its value comes from:

<!-- from: adapters/libreoffice/yantrik_libreoffice/surface.py -->
```python
            cells: Annotated[dict, "Each cell and what goes in it, like {\"A1\": \"Total\", "
                                   "\"B1\": 42, \"C1\": \"=B1*2\"}: a number, text, a formula "
                                   "starting with =, or null to empty it. Text that should start "
                                   "with = starts with ' instead"],
```

Never take a secret as an argument — a password, a PIN, a token. Arguments are what an approval
card draws and the audit log keeps; `yos check` fails a surface with a parameter named like one.

**Refusals** are sentences a mind acts on: what was wrong, and what to do instead.

<!-- from: adapters/libreoffice/yantrik_libreoffice/office.py -->
```python
            raise OfficeError("%d documents are open (%s); say which with `document`"
                              % (len(docs), ", ".join(names)))
```

The dispatch's own refusals follow the same rule: `` `add` argument `count` must be an integer, and
a string arrived`` names the argument and the kind that arrived — never the value, which might be
a PIN.

## `expected_seconds`

Most acts answer in a moment. When one usually takes longer — a render, an export, a command whose
exit code is the answer — declare how long, in whole seconds: `.expected_seconds(20)` in Rust,
`expected_seconds=20` in Python. It is published in `describe`, and a client should size its
timeout by it rather than guess one number for every action on the machine. (Today `yos act` waits
40 seconds for any act and does not read it yet; declare it anyway, for the clients that will.)

## Settles later, or answered later

Two different things, and a mind needs to know which.

**Settles later** means the act only *starts* the work. The answer comes at once and says
`settled: false`; the mind must not report the work as done, and watches `describe` for the
outcome — so `describe` has to show it. Declare it with `settles="later"` in Python or `.defers()`
in Rust. The Python example's `clear` empties its list over a few seconds, and its state says
`clearing` until it has:

<!-- from: examples/hello_surface.py -->
```python
@surface.action("clear", grade="sensitive", settles="later", expected_seconds=5)
```

**Answered later** means the caller is owed the *result* — the file an export wrote, a command's
exit code — and the work is too slow to do while the program is held. The handler checks what it
can, then returns the rest of the work (`Later(work)` in Python, `answer_later` in Rust): the
program is released at once, the caller's reply waits for the work, and the answer is the work's
own result, with the view read again after it. It settles on return, because when the reply comes
the work is done. LibreOffice's `export_pdf`:

<!-- from: adapters/libreoffice/yantrik_libreoffice/surface.py -->
```python
        # Checked in this turn — the document, the path — and written after it, so a long export
        # holds up nobody reading the surface in the meantime. The answer waits for the file.
        _, work = _answer(lambda: serving().export_pdf_plan(path, document))
        return Later(lambda: _answer(work))
```

## What makes a mind choose the right action

- A **purpose** line that says what the program is *for*, written against the program it is most
  likely to be confused with (the desktop's yDoc keeps Markdown documents; LibreOffice's purpose
  says *office files*).
- **Distinct** names and descriptions: no two actions a mind could mistake for each other.
- **Parameters that name what describe lists**, so the mind copies an identifier instead of
  inventing one.
- **Honest grades**. A mind plans around the cards a person will see; an action graded lower than
  it deserves surprises the person, and one graded higher teaches them to press Allow without
  reading ([Choosing a grade](grades.md)).
- **Refusals that say what to do next**, so a mistake costs one turn, not five.
- **The answer carries the view**, so the mind's next decision is made on what is there now.

## A whole describe

The [Python template](../../templates/python-surface/my_surface.py)'s, as `app.describe` answers
it, before anything is added:

<!-- output: python-template-describe -->
```json
{
  "app": "my-surface",
  "protocol": 1,
  "summary": "My Surface — 0 tasks, 0 done",
  "state": {"tasks": []},
  "revision": "245218f2c3e15db8",
  "actions": [
    {
      "name": "add",
      "description": "Put a task on the list",
      "permission": "standard",
      "settles": "on return",
      "parameters": {
        "type": "object",
        "properties": {
          "title": {"type": "string", "description": "What needs doing, in a few words"},
          "priority": {"type": "string", "description": "How soon it matters",
                       "enum": ["low", "normal", "high"], "default": "normal"}
        },
        "required": ["title"]
      }
    },
    {
      "name": "complete",
      "description": "Mark a task done, or not done again with `done` false",
      "permission": "standard",
      "settles": "on return",
      "parameters": {
        "type": "object",
        "properties": {
          "index": {"type": "integer", "description": "The task's `index`, as describe lists it"},
          "done": {"type": "boolean", "description": "false to mark it not done", "default": true}
        },
        "required": ["index"]
      }
    },
    {
      "name": "find",
      "description": "Search the tasks' titles, changing nothing",
      "permission": "safe",
      "settles": "on return",
      "parameters": {
        "type": "object",
        "properties": {
          "query": {"type": "string", "description": "Words to look for, in any case"}
        },
        "required": ["query"]
      }
    },
    {
      "name": "remove",
      "description": "Take a task off the list. It cannot be undone",
      "permission": "sensitive",
      "settles": "on return",
      "parameters": {
        "type": "object",
        "properties": {
          "index": {"type": "integer", "description": "The task's `index`, as describe lists it"}
        },
        "required": ["index"]
      }
    }
  ]
}
```

The same template in Rust publishes the same describe, key for key: the two SDKs are one dispatch.
