#!/usr/bin/env python3
"""Generate synthetic training data for FILES tool family (250 examples).
Output: batch_tools_02_files.jsonl
"""
import json, random, os
from pathlib import Path

random.seed(42)
OUT_DIR = Path(__file__).parent

# ---------------------------------------------------------------------------
# Bond stages
# ---------------------------------------------------------------------------
BOND_PROMPTS = {
    "stranger": (
        "You are Yantrik, a personal AI companion on the user's desktop. "
        "Bond stage: STRANGER. Be helpful, polite, slightly reserved. "
        "Do not assume familiarity. Use full sentences. No filler phrases, no emoji."
    ),
    "acquaintance": (
        "You are Yantrik, a personal AI companion on the user's desktop. "
        "Bond stage: ACQUAINTANCE. Be friendly and warm. You know basic preferences. "
        "Concise, natural contractions. No filler phrases, no emoji."
    ),
    "trusted": (
        "You are Yantrik, a personal AI companion on the user's desktop. "
        "Bond stage: TRUSTED. Casual and direct. Reference shared history when relevant. "
        "Offer opinions. No filler phrases, no emoji."
    ),
    "deep": (
        "You are Yantrik, a personal AI companion on the user's desktop. "
        "Bond stage: DEEP. Intimate, unfiltered. Anticipate needs. "
        "Use shorthand and inside references. No filler phrases, no emoji."
    ),
}
BOND_STAGES = list(BOND_PROMPTS.keys())

# ---------------------------------------------------------------------------
# Data pools
# ---------------------------------------------------------------------------
DIRS = [
    "/home/user/Downloads", "/home/user/Documents", "/home/user/Desktop",
    "/home/user/Projects", "/home/user/Pictures", "/home/user/Music",
    "/home/user/Videos", "/home/user/.config", "/tmp", "/var/log",
    "/home/user/Projects/webapp", "/home/user/Projects/api-server",
    "/home/user/Documents/work", "/home/user/Documents/personal",
    "/home/user/Projects/ml-pipeline", "/home/user/backup",
]
CODE_FILES = [
    ("main.rs", "rust"), ("lib.rs", "rust"), ("mod.rs", "rust"),
    ("app.py", "python"), ("server.py", "python"), ("utils.py", "python"),
    ("index.ts", "typescript"), ("app.tsx", "typescript"), ("config.ts", "typescript"),
    ("main.go", "go"), ("handler.go", "go"), ("Makefile", "makefile"),
    ("Dockerfile", "dockerfile"), ("docker-compose.yml", "yaml"),
    ("README.md", "markdown"), (".env", "env"), ("package.json", "json"),
    ("Cargo.toml", "toml"), ("config.yaml", "yaml"), (".gitignore", "gitignore"),
]
DOC_FILES = [
    "report.pdf", "notes.txt", "meeting-minutes.md", "budget.xlsx",
    "proposal.docx", "invoice.pdf", "resume.pdf", "README.md",
    "CHANGELOG.md", "LICENSE", "todo.txt", "ideas.md",
]
MEDIA_FILES = [
    ("photo.jpg", "4.5MB"), ("screenshot.png", "1.2MB"), ("wallpaper.png", "8.3MB"),
    ("vacation.jpg", "6.1MB"), ("diagram.svg", "45KB"), ("logo.png", "128KB"),
    ("song.mp3", "5.2MB"), ("podcast.mp3", "48MB"), ("video.mp4", "250MB"),
    ("recording.wav", "32MB"), ("clip.mp4", "15MB"),
]
ARCHIVE_FILES = ["backup.tar.gz", "project.zip", "data.tar.bz2", "logs.tar.gz", "export.zip"]
EXTENSIONS = [".rs", ".py", ".ts", ".go", ".js", ".json", ".yaml", ".toml", ".md", ".txt"]
SEARCH_TERMS = [
    "TODO", "FIXME", "error", "panic", "unwrap", "async fn", "import",
    "def ", "class ", "struct ", "fn main", "password", "api_key",
    "localhost", "127.0.0.1", "deprecated", "unsafe", "pub fn",
]
PERMISSIONS = ["644", "755", "600", "700", "664", "775"]
OWNERS = ["user:user", "root:root", "user:staff", "www-data:www-data"]
SIZES = ["128B", "1.2KB", "45KB", "256KB", "1.5MB", "12MB", "128MB", "1.2GB"]
HASH_ALGOS = ["sha256", "md5", "sha1"]

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------
def rand_bond(): return random.choice(BOND_STAGES)
def rand_id(): return f"call_{random.randint(1,999999)}"
def rand_size(): return random.choice(SIZES)
def rand_date():
    return f"2026-{random.randint(1,12):02d}-{random.randint(1,28):02d}T{random.randint(0,23):02d}:{random.randint(0,59):02d}:00"
def rand_dir(): return random.choice(DIRS)
def rand_code(): return random.choice(CODE_FILES)
def rand_doc(): return random.choice(DOC_FILES)
def rand_perm(): return random.choice(PERMISSIONS)
def rpath(*parts): return "/".join(parts)

def tc(name, args):
    return {"id": rand_id(), "type": "function",
            "function": {"name": name, "arguments": json.dumps(args, ensure_ascii=False)}}

def tool_msg(cid, content):
    return {"role": "tool", "tool_call_id": cid, "content": json.dumps(content, ensure_ascii=False)}

def asst_tc(calls): return {"role": "assistant", "content": None, "tool_calls": calls}
def asst(text): return {"role": "assistant", "content": text}
def usr(text): return {"role": "user", "content": text}
def sys(bond): return {"role": "system", "content": BOND_PROMPTS[bond]}

def line(msgs, bond, tools, scenario="task_request"):
    return json.dumps({"messages": msgs, "metadata": {
        "bond_stage": bond, "tools_used": tools, "scenario_type": scenario
    }}, ensure_ascii=False)

def file_list(names_sizes):
    return [{"name": n, "size": s, "modified": rand_date()} for n, s in names_sizes]

# ---------------------------------------------------------------------------
# Single-tool generators — returns (bond, msgs_list, tool_names)
# ---------------------------------------------------------------------------
def gen_list_files(variant=0):
    b = rand_bond()
    d = rand_dir()
    prompts = [
        f"What's in {d}?", f"Show me the files in {d}", f"List {d}",
        f"What do I have in {d}?", f"Show contents of {d}",
    ]
    c = tc("list_files", {"path": d})
    nf = random.randint(2, 8)
    items = [(random.choice(DOC_FILES + [f[0] for f in CODE_FILES]), rand_size()) for _ in range(nf)]
    r = tool_msg(c["id"], {"files": file_list(items), "count": nf})
    summary_items = ", ".join(f"{n} ({s})" for n, s in items[:4])
    resp = f"{'Found' if b in ('stranger','acquaintance') else 'Got'} {nf} items in {d}:\n" + "\n".join(f"- {n} ({s})" for n, s in items)
    return b, [sys(b), usr(random.choice(prompts)), asst_tc([c]), r, asst(resp)], ["list_files"]

def gen_read_file(variant=0):
    b = rand_bond()
    fname, ftype = rand_code()
    d = rand_dir()
    p = rpath(d, fname)
    prompts = [f"Show me {p}", f"Read {p}", f"What's in {p}?", f"Open {p}", f"Cat {p}"]
    c = tc("read_file", {"path": p})
    content_samples = {
        "rust": 'fn main() {\n    println!("Hello, world!");\n}',
        "python": 'def main():\n    print("Hello, world!")\n\nif __name__ == "__main__":\n    main()',
        "typescript": 'export function handler(req: Request): Response {\n  return new Response("OK");\n}',
        "go": 'package main\n\nimport "fmt"\n\nfunc main() {\n\tfmt.Println("Hello")\n}',
        "json": '{\n  "name": "my-project",\n  "version": "1.0.0"\n}',
        "yaml": 'server:\n  port: 8080\n  host: 0.0.0.0',
        "toml": '[package]\nname = "my-project"\nversion = "0.1.0"',
    }
    content = content_samples.get(ftype, "# Sample content\nLine 1\nLine 2")
    r = tool_msg(c["id"], {"content": content, "lines": content.count("\n") + 1, "size": rand_size()})
    resp = f"Here's {fname}:\n\n```{ftype}\n{content}\n```"
    return b, [sys(b), usr(random.choice(prompts)), asst_tc([c]), r, asst(resp)], ["read_file"]

def gen_write_file(variant=0):
    b = rand_bond()
    d = rand_dir()
    templates = [
        ("config.yaml", "server:\n  port: 8080\n  debug: false", "Create a config file at {p}"),
        ("notes.txt", "Meeting notes - Project sync\n- Review sprint goals\n- Assign tasks", "Write meeting notes to {p}"),
        ("script.sh", "#!/bin/bash\necho 'Starting backup...'\ntar czf backup.tar.gz /home/user/Documents", "Create a backup script at {p}"),
        (".gitignore", "target/\nnode_modules/\n*.log\n.env", "Make a gitignore at {p}"),
        ("Dockerfile", "FROM python:3.11-slim\nWORKDIR /app\nCOPY . .\nRUN pip install -r requirements.txt\nCMD [\"python\", \"app.py\"]", "Write a Dockerfile to {p}"),
    ]
    fname, content, prompt_tpl = random.choice(templates)
    p = rpath(d, fname)
    c = tc("write_file", {"path": p, "content": content})
    r = tool_msg(c["id"], {"written": True, "bytes": len(content), "path": p})
    resps = [f"Created {p}.", f"File written to {p}.", f"Done. {fname} is ready at {p}."]
    return b, [sys(b), usr(prompt_tpl.format(p=p)), asst_tc([c]), r, asst(random.choice(resps))], ["write_file"]

def gen_search_files(variant=0):
    b = rand_bond()
    d = rand_dir()
    fname = random.choice(DOC_FILES + [f[0] for f in CODE_FILES])
    prompts = [f"Find {fname}", f"Where is {fname}?", f"Search for {fname}", f"Locate {fname} on my system"]
    c = tc("search_files", {"name": fname, "path": "/home/user"})
    nf = random.randint(1, 4)
    matches = [rpath(random.choice(DIRS), fname) for _ in range(nf)]
    r = tool_msg(c["id"], {"matches": matches, "count": nf})
    if nf == 1:
        resp = f"Found it: {matches[0]}"
    else:
        resp = f"Found {nf} matches:\n" + "\n".join(f"- {m}" for m in matches)
    return b, [sys(b), usr(random.choice(prompts)), asst_tc([c]), r, asst(resp)], ["search_files"]

def gen_edit_file(variant=0):
    b = rand_bond()
    d = rand_dir()
    edits = [
        ("config.yaml", "port: 8080", "port: 3000", "Change the port to 3000 in {p}"),
        ("main.rs", 'println!("Hello")', 'println!("Hello, Yantrik!")', "Update the greeting in {p}"),
        ("app.py", "DEBUG = True", "DEBUG = False", "Disable debug mode in {p}"),
        (".env", "API_KEY=old_key_123", "API_KEY=new_key_456", "Update the API key in {p}"),
        ("docker-compose.yml", "image: nginx:1.20", "image: nginx:1.25", "Upgrade nginx version in {p}"),
    ]
    fname, old, new, prompt_tpl = random.choice(edits)
    p = rpath(d, fname)
    c = tc("edit_file", {"path": p, "old_text": old, "new_text": new})
    r = tool_msg(c["id"], {"edited": True, "replacements": 1, "path": p})
    resps = [f"Updated {fname}. Changed `{old}` to `{new}`.", f"Done. {fname} has been edited.", f"Applied the change to {p}."]
    return b, [sys(b), usr(prompt_tpl.format(p=p)), asst_tc([c]), r, asst(random.choice(resps))], ["edit_file"]

def gen_grep(variant=0):
    b = rand_bond()
    d = rand_dir()
    term = random.choice(SEARCH_TERMS)
    ext = random.choice(EXTENSIONS)
    prompts = [f"Search for '{term}' in {d}", f"Grep for {term} in {d}", f"Find all {term} references in {d}",
               f"Where is '{term}' used in {d}?"]
    c = tc("grep", {"pattern": term, "path": d, "glob": f"*{ext}"})
    nm = random.randint(1, 6)
    matches = [{"file": rpath(d, f"file{i}{ext}"), "line": random.randint(1, 200),
                "text": f"    {term} // found here"} for i in range(nm)]
    r = tool_msg(c["id"], {"matches": matches, "count": nm})
    resp = f"Found {nm} match{'es' if nm != 1 else ''} for '{term}':\n" + "\n".join(
        f"- {m['file']}:{m['line']}: {m['text'].strip()}" for m in matches[:5])
    return b, [sys(b), usr(random.choice(prompts)), asst_tc([c]), r, asst(resp)], ["grep"]

def gen_glob(variant=0):
    b = rand_bond()
    d = rand_dir()
    ext = random.choice(EXTENSIONS)
    prompts = [f"Find all {ext} files in {d}", f"List {ext} files under {d}", f"Show me every {ext} file in {d}"]
    c = tc("glob", {"pattern": f"**/*{ext}", "path": d})
    nf = random.randint(2, 8)
    files = [rpath(d, f"src/file{i}{ext}") for i in range(nf)]
    r = tool_msg(c["id"], {"files": files, "count": nf})
    resp = f"{nf} {ext} files found:\n" + "\n".join(f"- {f}" for f in files)
    return b, [sys(b), usr(random.choice(prompts)), asst_tc([c]), r, asst(resp)], ["glob"]

def gen_mkdir(variant=0):
    b = rand_bond()
    d = rand_dir()
    names = ["components", "tests", "utils", "backup", "logs", "data", "assets", "docs", "scripts", "migrations"]
    name = random.choice(names)
    p = rpath(d, name)
    prompts = [f"Create a {name} directory in {d}", f"Make folder {p}", f"mkdir {p}"]
    c = tc("mkdir", {"path": p})
    r = tool_msg(c["id"], {"created": True, "path": p})
    return b, [sys(b), usr(random.choice(prompts)), asst_tc([c]), r, asst(f"Created directory {p}.")], ["mkdir"]

def gen_rmdir(variant=0):
    b = rand_bond()
    d = rand_dir()
    name = random.choice(["old_backup", "tmp", "cache", "__pycache__", "node_modules", ".next", "dist", "build"])
    p = rpath(d, name)
    prompts = [f"Remove the {name} directory in {d}", f"Delete {p}", f"Clean up {p}"]
    c = tc("rmdir", {"path": p, "recursive": True})
    r = tool_msg(c["id"], {"removed": True, "path": p, "files_deleted": random.randint(5, 200)})
    nd = random.randint(5, 200)
    return b, [sys(b), usr(random.choice(prompts)), asst_tc([c]), r, asst(f"Removed {p} and its contents.")], ["rmdir"]

def gen_move_file(variant=0):
    b = rand_bond()
    src_d, dst_d = random.sample(DIRS, 2)
    fname = random.choice(DOC_FILES)
    src = rpath(src_d, fname)
    dst = rpath(dst_d, fname)
    prompts = [f"Move {src} to {dst_d}", f"Move {fname} from {src_d} to {dst_d}", f"mv {src} {dst}"]
    c = tc("move_file", {"source": src, "destination": dst})
    r = tool_msg(c["id"], {"moved": True, "from": src, "to": dst})
    return b, [sys(b), usr(random.choice(prompts)), asst_tc([c]), r, asst(f"Moved {fname} to {dst_d}.")], ["move_file"]

def gen_copy_file(variant=0):
    b = rand_bond()
    src_d, dst_d = random.sample(DIRS, 2)
    fname = random.choice(DOC_FILES)
    src = rpath(src_d, fname)
    dst = rpath(dst_d, fname)
    prompts = [f"Copy {src} to {dst_d}", f"Duplicate {fname} to {dst_d}", f"cp {src} {dst}"]
    c = tc("copy_file", {"source": src, "destination": dst})
    r = tool_msg(c["id"], {"copied": True, "from": src, "to": dst, "size": rand_size()})
    return b, [sys(b), usr(random.choice(prompts)), asst_tc([c]), r, asst(f"Copied {fname} to {dst_d}.")], ["copy_file"]

def gen_file_info(variant=0):
    b = rand_bond()
    fname, ftype = rand_code()
    d = rand_dir()
    p = rpath(d, fname)
    prompts = [f"File info for {p}", f"Details about {p}", f"What can you tell me about {p}?", f"stat {p}"]
    c = tc("file_info", {"path": p})
    info = {"path": p, "size": rand_size(), "type": ftype, "permissions": rand_perm(),
            "owner": random.choice(OWNERS), "modified": rand_date(), "created": rand_date()}
    r = tool_msg(c["id"], info)
    resp = f"{fname}: {info['size']}, {ftype} file, permissions {info['permissions']}, modified {info['modified'][:10]}."
    return b, [sys(b), usr(random.choice(prompts)), asst_tc([c]), r, asst(resp)], ["file_info"]

def gen_file_hash(variant=0):
    b = rand_bond()
    fname = random.choice(DOC_FILES)
    p = rpath(rand_dir(), fname)
    algo = random.choice(HASH_ALGOS)
    prompts = [f"Get the {algo} hash of {p}", f"Checksum {p}", f"Hash {p} with {algo}"]
    c = tc("file_hash", {"path": p, "algorithm": algo})
    h = "".join(random.choices("0123456789abcdef", k=64 if algo == "sha256" else 32))
    r = tool_msg(c["id"], {"hash": h, "algorithm": algo, "path": p})
    return b, [sys(b), usr(random.choice(prompts)), asst_tc([c]), r, asst(f"{algo.upper()} hash of {fname}: `{h}`")], ["file_hash"]

def gen_compress(variant=0):
    b = rand_bond()
    d = rand_dir()
    fmt = random.choice(["tar.gz", "zip", "tar.bz2"])
    prompts = [f"Compress {d} into a {fmt}", f"Archive {d}", f"Zip up {d}"]
    out = f"/home/user/backup/{d.split('/')[-1]}.{fmt}"
    c = tc("compress", {"source": d, "destination": out, "format": fmt})
    r = tool_msg(c["id"], {"compressed": True, "path": out, "size": rand_size(), "files": random.randint(5, 50)})
    return b, [sys(b), usr(random.choice(prompts)), asst_tc([c]), r, asst(f"Compressed {d} to {out}.")], ["compress"]

def gen_decompress(variant=0):
    b = rand_bond()
    arc = random.choice(ARCHIVE_FILES)
    p = rpath("/home/user/Downloads", arc)
    dst = "/home/user/extracted"
    prompts = [f"Extract {p}", f"Decompress {p}", f"Unzip {p} to {dst}"]
    c = tc("decompress", {"source": p, "destination": dst})
    nf = random.randint(3, 30)
    r = tool_msg(c["id"], {"extracted": True, "destination": dst, "files": nf})
    return b, [sys(b), usr(random.choice(prompts)), asst_tc([c]), r, asst(f"Extracted {nf} files to {dst}.")], ["decompress"]

def gen_watch_file(variant=0):
    b = rand_bond()
    fname, _ = rand_code()
    p = rpath(rand_dir(), fname)
    prompts = [f"Watch {p} for changes", f"Monitor {p}", f"Notify me if {p} changes"]
    c = tc("watch_file", {"path": p})
    r = tool_msg(c["id"], {"watching": True, "path": p, "watch_id": f"w_{random.randint(100,999)}"})
    return b, [sys(b), usr(random.choice(prompts)), asst_tc([c]), r, asst(f"Now watching {p} for changes. I'll notify you when it's modified.")], ["watch_file"]

def gen_trash(variant=0):
    b = rand_bond()
    fname = random.choice(DOC_FILES)
    p = rpath(rand_dir(), fname)
    prompts = [f"Trash {p}", f"Delete {p}", f"Remove {fname}", f"Send {p} to trash"]
    c = tc("trash", {"path": p})
    tid = f"trash_{random.randint(1000,9999)}"
    r = tool_msg(c["id"], {"trashed": True, "path": p, "trash_id": tid})
    resps = [f"Moved {fname} to trash. You can restore it if needed.", f"{fname} trashed.", f"Deleted {fname}. It's recoverable from trash."]
    return b, [sys(b), usr(random.choice(prompts)), asst_tc([c]), r, asst(random.choice(resps))], ["trash"]

def gen_restore_trash(variant=0):
    b = rand_bond()
    fname = random.choice(DOC_FILES)
    tid = f"trash_{random.randint(1000,9999)}"
    orig = rpath(rand_dir(), fname)
    prompts = [f"Restore {fname} from trash", f"Undelete {fname}", f"Recover {fname}"]
    c = tc("restore_trash", {"trash_id": tid})
    r = tool_msg(c["id"], {"restored": True, "path": orig, "trash_id": tid})
    return b, [sys(b), usr(random.choice(prompts)), asst_tc([c]), r, asst(f"Restored {fname} to {orig}.")], ["restore_trash"]

def gen_list_trash(variant=0):
    b = rand_bond()
    prompts = ["What's in the trash?", "Show me deleted files", "List trash", "What did I delete recently?"]
    c = tc("list_trash", {})
    nf = random.randint(1, 5)
    items = [{"name": random.choice(DOC_FILES), "deleted": rand_date(), "trash_id": f"trash_{random.randint(1000,9999)}",
              "original_path": rpath(rand_dir(), random.choice(DOC_FILES)), "size": rand_size()} for _ in range(nf)]
    r = tool_msg(c["id"], {"items": items, "count": nf})
    resp = f"{nf} item{'s' if nf != 1 else ''} in trash:\n" + "\n".join(
        f"- {it['name']} ({it['size']}, deleted {it['deleted'][:10]})" for it in items)
    return b, [sys(b), usr(random.choice(prompts)), asst_tc([c]), r, asst(resp)], ["list_trash"]

def gen_disk_space(variant=0):
    b = rand_bond()
    prompts = ["How much disk space do I have?", "Check disk usage", "df", "Am I running low on storage?"]
    c = tc("disk_space", {})
    info = {"total": "500GB", "used": f"{random.randint(100,450)}GB", "free": f"{random.randint(20,200)}GB",
            "percent_used": random.randint(30, 92),
            "mounts": [{"path": "/", "total": "500GB", "used": f"{random.randint(100,400)}GB"}]}
    r = tool_msg(c["id"], info)
    resp = f"Disk usage: {info['used']} of {info['total']} used ({info['percent_used']}%). {info['free']} free."
    return b, [sys(b), usr(random.choice(prompts)), asst_tc([c]), r, asst(resp)], ["disk_space"]

def gen_mount_info(variant=0):
    b = rand_bond()
    prompts = ["Show mounted filesystems", "What drives are mounted?", "mount info", "List mounts"]
    c = tc("mount_info", {})
    mounts = [{"device": "/dev/sda1", "mount": "/", "type": "ext4", "size": "500GB"},
              {"device": "/dev/sdb1", "mount": "/home", "type": "ext4", "size": "1TB"}]
    if random.random() > 0.5:
        mounts.append({"device": "/dev/sdc1", "mount": "/mnt/usb", "type": "vfat", "size": "32GB"})
    r = tool_msg(c["id"], {"mounts": mounts})
    resp = "Mounted filesystems:\n" + "\n".join(f"- {m['device']} on {m['mount']} ({m['type']}, {m['size']})" for m in mounts)
    return b, [sys(b), usr(random.choice(prompts)), asst_tc([c]), r, asst(resp)], ["mount_info"]

def gen_symlink(variant=0):
    b = rand_bond()
    d = rand_dir()
    fname, _ = rand_code()
    target = rpath(d, fname)
    link = rpath("/home/user", fname)
    prompts = [f"Create a symlink from {link} to {target}", f"Link {link} -> {target}", f"Symlink {target} as {link}"]
    c = tc("symlink", {"target": target, "link": link})
    r = tool_msg(c["id"], {"created": True, "link": link, "target": target})
    return b, [sys(b), usr(random.choice(prompts)), asst_tc([c]), r, asst(f"Symlink created: {link} -> {target}")], ["symlink"]

def gen_permissions(variant=0):
    b = rand_bond()
    fname, _ = rand_code()
    p = rpath(rand_dir(), fname)
    perm = rand_perm()
    is_get = random.random() > 0.5
    if is_get:
        prompts = [f"What are the permissions on {p}?", f"Check permissions for {p}"]
        c = tc("permissions", {"path": p})
        r = tool_msg(c["id"], {"path": p, "mode": perm, "readable": True, "writable": True, "executable": perm in ("755", "700", "775")})
        resp = f"Permissions on {fname}: {perm} ({'rwx' if perm.startswith('7') else 'rw-'})"
    else:
        prompts = [f"Set {p} to {perm}", f"chmod {perm} {p}", f"Make {p} executable"]
        c = tc("permissions", {"path": p, "mode": perm})
        r = tool_msg(c["id"], {"updated": True, "path": p, "mode": perm})
        resp = f"Permissions on {fname} set to {perm}."
    return b, [sys(b), usr(random.choice(prompts)), asst_tc([c]), r, asst(resp)], ["permissions"]

def gen_ownership(variant=0):
    b = rand_bond()
    p = rpath(rand_dir(), random.choice(DOC_FILES))
    owner = random.choice(OWNERS)
    prompts = [f"Change owner of {p} to {owner}", f"chown {owner} {p}", f"Who owns {p}?"]
    c = tc("ownership", {"path": p, "owner": owner})
    r = tool_msg(c["id"], {"updated": True, "path": p, "owner": owner})
    return b, [sys(b), usr(random.choice(prompts)), asst_tc([c]), r, asst(f"Ownership of {p} set to {owner}.")], ["ownership"]

def gen_find_duplicates(variant=0):
    b = rand_bond()
    d = rand_dir()
    prompts = [f"Find duplicate files in {d}", f"Any duplicates in {d}?", f"Check for duplicate files in {d}"]
    c = tc("find_duplicates", {"path": d})
    nd = random.randint(0, 4)
    groups = []
    for _ in range(nd):
        f1 = rpath(d, random.choice(DOC_FILES))
        f2 = rpath(d, "copy_" + random.choice(DOC_FILES))
        groups.append({"hash": "".join(random.choices("0123456789abcdef", k=16)), "files": [f1, f2], "size": rand_size()})
    r = tool_msg(c["id"], {"groups": groups, "duplicate_groups": nd})
    if nd == 0:
        resp = f"No duplicate files found in {d}."
    else:
        resp = f"Found {nd} group{'s' if nd != 1 else ''} of duplicates:\n" + "\n".join(
            f"- {', '.join(g['files'])} ({g['size']})" for g in groups)
    return b, [sys(b), usr(random.choice(prompts)), asst_tc([c]), r, asst(resp)], ["find_duplicates"]

def gen_recent_files(variant=0):
    b = rand_bond()
    n = random.choice([5, 10, 20])
    prompts = [f"Show my {n} most recent files", "What did I work on recently?", "Recent files", "Last modified files"]
    c = tc("recent_files", {"limit": n})
    nf = min(n, random.randint(3, 10))
    files = [{"path": rpath(rand_dir(), random.choice(DOC_FILES + [f[0] for f in CODE_FILES])),
              "modified": rand_date(), "size": rand_size()} for _ in range(nf)]
    r = tool_msg(c["id"], {"files": files, "count": nf})
    resp = f"Your {nf} most recently modified files:\n" + "\n".join(
        f"- {f['path']} (modified {f['modified'][:10]})" for f in files[:8])
    return b, [sys(b), usr(random.choice(prompts)), asst_tc([c]), r, asst(resp)], ["recent_files"]

def gen_large_files(variant=0):
    b = rand_bond()
    d = random.choice(["/home/user", rand_dir()])
    prompts = [f"Find large files in {d}", "What's eating my disk space?", f"Show biggest files in {d}",
               "Find files over 100MB"]
    c = tc("large_files", {"path": d, "limit": 10, "min_size": "10MB"})
    nf = random.randint(2, 7)
    big_sizes = ["128MB", "256MB", "512MB", "1.2GB", "2.4GB", "48MB", "95MB"]
    files = [{"path": rpath(d, random.choice(MEDIA_FILES)[0] if random.random() > 0.3 else random.choice(ARCHIVE_FILES)),
              "size": random.choice(big_sizes)} for _ in range(nf)]
    r = tool_msg(c["id"], {"files": files, "count": nf})
    resp = f"Largest files in {d}:\n" + "\n".join(f"- {f['path']} ({f['size']})" for f in files)
    return b, [sys(b), usr(random.choice(prompts)), asst_tc([c]), r, asst(resp)], ["large_files"]

def gen_file_type(variant=0):
    b = rand_bond()
    fname = random.choice(DOC_FILES + [f[0] for f in CODE_FILES])
    p = rpath(rand_dir(), fname)
    prompts = [f"What type of file is {p}?", f"file {p}", f"Identify {p}"]
    c = tc("file_type", {"path": p})
    types = {"pdf": "application/pdf", "txt": "text/plain", "md": "text/markdown", "xlsx": "application/vnd.openxmlformats",
             "docx": "application/vnd.openxmlformats", "rs": "text/x-rust", "py": "text/x-python", "ts": "text/typescript",
             "go": "text/x-go", "json": "application/json", "yaml": "text/yaml", "toml": "text/toml"}
    ext = fname.rsplit(".", 1)[-1] if "." in fname else "txt"
    mime = types.get(ext, "application/octet-stream")
    r = tool_msg(c["id"], {"path": p, "mime": mime, "description": f"{ext.upper()} file"})
    return b, [sys(b), usr(random.choice(prompts)), asst_tc([c]), r, asst(f"{fname} is a {ext.upper()} file ({mime}).")], ["file_type"]

def gen_count_lines(variant=0):
    b = rand_bond()
    fname, ftype = rand_code()
    p = rpath(rand_dir(), fname)
    prompts = [f"How many lines in {p}?", f"Count lines in {p}", f"wc -l {p}", f"Line count for {p}"]
    c = tc("count_lines", {"path": p})
    nl = random.randint(10, 2000)
    r = tool_msg(c["id"], {"path": p, "lines": nl, "non_empty": nl - random.randint(5, nl // 4)})
    return b, [sys(b), usr(random.choice(prompts)), asst_tc([c]), r, asst(f"{fname} has {nl} lines.")], ["count_lines"]

def gen_diff_files(variant=0):
    b = rand_bond()
    d = rand_dir()
    fname, ftype = rand_code()
    f1 = rpath(d, fname)
    f2 = rpath(d, f"{fname}.bak")
    prompts = [f"Diff {f1} and {f2}", f"Compare {f1} with {f2}", f"What changed between {f1} and {f2}?"]
    c = tc("diff_files", {"file_a": f1, "file_b": f2})
    nd = random.randint(1, 5)
    diffs = [{"line": random.randint(1, 100), "old": "old content", "new": "new content"} for _ in range(nd)]
    r = tool_msg(c["id"], {"differences": diffs, "count": nd, "identical": False})
    resp = f"{nd} difference{'s' if nd != 1 else ''} between the files:\n" + "\n".join(
        f"- Line {d['line']}: `{d['old']}` -> `{d['new']}`" for d in diffs[:5])
    return b, [sys(b), usr(random.choice(prompts)), asst_tc([c]), r, asst(resp)], ["diff_files"]

# ---------------------------------------------------------------------------
# Multi-tool generators (18% of 250 = 45)
# ---------------------------------------------------------------------------
MULTI_SCENARIOS = [
    # (user_prompt, tool_sequence)
    lambda: _multi_list_read(),
    lambda: _multi_grep_edit(),
    lambda: _multi_search_read(),
    lambda: _multi_find_dup_trash(),
    lambda: _multi_large_trash(),
    lambda: _multi_glob_count(),
    lambda: _multi_copy_verify(),
    lambda: _multi_mkdir_write(),
    lambda: _multi_diff_edit(),
    lambda: _multi_info_permissions(),
    lambda: _multi_list_compress(),
    lambda: _multi_decompress_list(),
    lambda: _multi_read_write(),
    lambda: _multi_grep_read(),
    lambda: _multi_disk_large(),
]

def _multi_list_read():
    b = rand_bond()
    d = rand_dir()
    fname, ftype = rand_code()
    # Step 1: list
    c1 = tc("list_files", {"path": d})
    items = file_list([(fname, rand_size()), (random.choice(DOC_FILES), rand_size())])
    r1 = tool_msg(c1["id"], {"files": items, "count": len(items)})
    # Step 2: read
    p = rpath(d, fname)
    c2 = tc("read_file", {"path": p})
    content = "fn main() {\n    println!(\"Hello\");\n}"
    r2 = tool_msg(c2["id"], {"content": content, "lines": 3})
    resp1 = f"Found {len(items)} files. Let me read {fname}."
    resp2 = f"Here's {fname}:\n\n```{ftype}\n{content}\n```"
    return b, [sys(b), usr(f"Show me what's in {d} and open the first code file"),
               asst_tc([c1]), r1, asst(resp1), asst_tc([c2]), r2, asst(resp2)], ["list_files", "read_file"]

def _multi_grep_edit():
    b = rand_bond()
    d = rand_dir()
    term = "TODO"
    fname = "main.rs"
    p = rpath(d, fname)
    c1 = tc("grep", {"pattern": term, "path": d})
    r1 = tool_msg(c1["id"], {"matches": [{"file": p, "line": 42, "text": "    // TODO: implement error handling"}], "count": 1})
    c2 = tc("edit_file", {"path": p, "old_text": "// TODO: implement error handling", "new_text": "// Error handling implemented"})
    r2 = tool_msg(c2["id"], {"edited": True, "replacements": 1})
    return b, [sys(b), usr(f"Find all TODOs in {d} and mark the first one as done"),
               asst_tc([c1]), r1, asst(f"Found 1 TODO in {fname} at line 42. Marking it done."),
               asst_tc([c2]), r2, asst("Done. Updated the TODO comment.")], ["grep", "edit_file"]

def _multi_search_read():
    b = rand_bond()
    fname = random.choice(DOC_FILES)
    c1 = tc("search_files", {"name": fname, "path": "/home/user"})
    p = rpath(rand_dir(), fname)
    r1 = tool_msg(c1["id"], {"matches": [p], "count": 1})
    c2 = tc("read_file", {"path": p})
    r2 = tool_msg(c2["id"], {"content": "Meeting notes from last week.\n- Action items pending.", "lines": 2})
    return b, [sys(b), usr(f"Find {fname} and show me what's in it"),
               asst_tc([c1]), r1, asst(f"Found it at {p}. Reading now."),
               asst_tc([c2]), r2, asst(f"Contents of {fname}:\n\nMeeting notes from last week.\n- Action items pending.")], ["search_files", "read_file"]

def _multi_find_dup_trash():
    b = rand_bond()
    d = rand_dir()
    f1 = rpath(d, "report.pdf")
    f2 = rpath(d, "report_copy.pdf")
    c1 = tc("find_duplicates", {"path": d})
    r1 = tool_msg(c1["id"], {"groups": [{"hash": "abc123", "files": [f1, f2], "size": "2.1MB"}], "duplicate_groups": 1})
    c2 = tc("trash", {"path": f2})
    r2 = tool_msg(c2["id"], {"trashed": True, "path": f2, "trash_id": "trash_5001"})
    return b, [sys(b), usr(f"Find and clean up duplicate files in {d}"),
               asst_tc([c1]), r1, asst(f"Found 1 duplicate pair: report.pdf and report_copy.pdf. Removing the copy."),
               asst_tc([c2]), r2, asst("Trashed report_copy.pdf. The original is kept.")], ["find_duplicates", "trash"]

def _multi_large_trash():
    b = rand_bond()
    d = "/home/user"
    c1 = tc("large_files", {"path": d, "limit": 5, "min_size": "100MB"})
    big = rpath(d, "Videos/old_recording.mp4")
    r1 = tool_msg(c1["id"], {"files": [{"path": big, "size": "1.2GB"}], "count": 1})
    c2 = tc("trash", {"path": big})
    r2 = tool_msg(c2["id"], {"trashed": True, "path": big, "trash_id": "trash_6001"})
    return b, [sys(b), usr("I need to free up space. Find big files and trash anything over 1GB"),
               asst_tc([c1]), r1, asst(f"Found old_recording.mp4 at 1.2GB. Trashing it."),
               asst_tc([c2]), r2, asst("Trashed old_recording.mp4. Freed up 1.2GB.")], ["large_files", "trash"]

def _multi_glob_count():
    b = rand_bond()
    d = rand_dir()
    c1 = tc("glob", {"pattern": "**/*.py", "path": d})
    files = [rpath(d, f"src/mod{i}.py") for i in range(4)]
    r1 = tool_msg(c1["id"], {"files": files, "count": 4})
    c2 = tc("count_lines", {"path": files[0]})
    r2 = tool_msg(c2["id"], {"path": files[0], "lines": 245, "non_empty": 210})
    return b, [sys(b), usr(f"How many Python files in {d} and how long is the first one?"),
               asst_tc([c1]), r1, asst(f"4 Python files found. Checking line count of {files[0]}."),
               asst_tc([c2]), r2, asst(f"4 .py files total. {files[0]} has 245 lines.")], ["glob", "count_lines"]

def _multi_copy_verify():
    b = rand_bond()
    fname = random.choice(DOC_FILES)
    src = rpath(rand_dir(), fname)
    dst = rpath("/home/user/backup", fname)
    c1 = tc("copy_file", {"source": src, "destination": dst})
    r1 = tool_msg(c1["id"], {"copied": True, "from": src, "to": dst, "size": "2.1MB"})
    c2a = tc("file_hash", {"path": src, "algorithm": "sha256"})
    c2b = tc("file_hash", {"path": dst, "algorithm": "sha256"})
    h = "".join(random.choices("0123456789abcdef", k=64))
    r2a = tool_msg(c2a["id"], {"hash": h, "algorithm": "sha256"})
    r2b = tool_msg(c2b["id"], {"hash": h, "algorithm": "sha256"})
    return b, [sys(b), usr(f"Copy {src} to backup and verify the copy"),
               asst_tc([c1]), r1, asst("Copied. Verifying integrity."),
               asst_tc([c2a, c2b]), r2a, r2b, asst("Verified. Both files have identical SHA256 hashes. Copy is intact.")], ["copy_file", "file_hash"]

def _multi_mkdir_write():
    b = rand_bond()
    d = rpath(rand_dir(), "new_project")
    c1 = tc("mkdir", {"path": d})
    r1 = tool_msg(c1["id"], {"created": True, "path": d})
    readme = rpath(d, "README.md")
    c2 = tc("write_file", {"path": readme, "content": "# New Project\n\nProject description goes here."})
    r2 = tool_msg(c2["id"], {"written": True, "bytes": 48, "path": readme})
    return b, [sys(b), usr(f"Create a new project directory at {d} with a README"),
               asst_tc([c1]), r1, asst("Directory created. Writing README."),
               asst_tc([c2]), r2, asst(f"Project scaffolded at {d} with a README.md.")], ["mkdir", "write_file"]

def _multi_diff_edit():
    b = rand_bond()
    d = rand_dir()
    f1 = rpath(d, "config.yaml")
    f2 = rpath(d, "config.yaml.bak")
    c1 = tc("diff_files", {"file_a": f1, "file_b": f2})
    r1 = tool_msg(c1["id"], {"differences": [{"line": 5, "old": "port: 8080", "new": "port: 3000"}], "count": 1, "identical": False})
    c2 = tc("edit_file", {"path": f1, "old_text": "port: 8080", "new_text": "port: 3000"})
    r2 = tool_msg(c2["id"], {"edited": True, "replacements": 1})
    return b, [sys(b), usr(f"Compare {f1} with the backup and apply the backup's changes"),
               asst_tc([c1]), r1, asst("One difference: port changed from 8080 to 3000. Applying."),
               asst_tc([c2]), r2, asst("Applied. Port is now 3000, matching the backup.")], ["diff_files", "edit_file"]

def _multi_info_permissions():
    b = rand_bond()
    fname = "deploy.sh"
    p = rpath(rand_dir(), fname)
    c1 = tc("file_info", {"path": p})
    r1 = tool_msg(c1["id"], {"path": p, "size": "2.4KB", "permissions": "644", "owner": "user:user", "modified": rand_date()})
    c2 = tc("permissions", {"path": p, "mode": "755"})
    r2 = tool_msg(c2["id"], {"updated": True, "path": p, "mode": "755"})
    return b, [sys(b), usr(f"Check if {fname} is executable and make it so if not"),
               asst_tc([c1]), r1, asst(f"{fname} has permissions 644 -- not executable. Fixing."),
               asst_tc([c2]), r2, asst(f"Set {fname} to 755. It's executable now.")], ["file_info", "permissions"]

def _multi_list_compress():
    b = rand_bond()
    d = rand_dir()
    c1 = tc("list_files", {"path": d})
    nf = random.randint(5, 12)
    items = [(random.choice(DOC_FILES), rand_size()) for _ in range(nf)]
    r1 = tool_msg(c1["id"], {"files": file_list(items), "count": nf})
    out = f"/home/user/backup/{d.split('/')[-1]}.tar.gz"
    c2 = tc("compress", {"source": d, "destination": out, "format": "tar.gz"})
    r2 = tool_msg(c2["id"], {"compressed": True, "path": out, "size": rand_size(), "files": nf})
    return b, [sys(b), usr(f"Show me what's in {d} then back it all up"),
               asst_tc([c1]), r1, asst(f"{nf} files in {d}. Compressing."),
               asst_tc([c2]), r2, asst(f"Backed up {nf} files to {out}.")], ["list_files", "compress"]

def _multi_decompress_list():
    b = rand_bond()
    arc = random.choice(ARCHIVE_FILES)
    src = rpath("/home/user/Downloads", arc)
    dst = "/home/user/extracted"
    c1 = tc("decompress", {"source": src, "destination": dst})
    r1 = tool_msg(c1["id"], {"extracted": True, "destination": dst, "files": 8})
    c2 = tc("list_files", {"path": dst})
    items = [(random.choice(DOC_FILES), rand_size()) for _ in range(4)]
    r2 = tool_msg(c2["id"], {"files": file_list(items), "count": 4})
    return b, [sys(b), usr(f"Extract {arc} and show me what's inside"),
               asst_tc([c1]), r1, asst("Extracted. Listing contents."),
               asst_tc([c2]), r2, asst(f"Extracted {arc}. Contents:\n" + "\n".join(f"- {n} ({s})" for n, s in items))], ["decompress", "list_files"]

def _multi_read_write():
    b = rand_bond()
    d = rand_dir()
    p = rpath(d, "config.yaml")
    c1 = tc("read_file", {"path": p})
    old_content = "server:\n  port: 8080\n  debug: true"
    r1 = tool_msg(c1["id"], {"content": old_content, "lines": 3})
    new_content = "server:\n  port: 8080\n  debug: false\n  workers: 4"
    c2 = tc("write_file", {"path": p, "content": new_content})
    r2 = tool_msg(c2["id"], {"written": True, "bytes": len(new_content), "path": p})
    return b, [sys(b), usr(f"Read {p}, disable debug, add workers: 4, and save"),
               asst_tc([c1]), r1, asst("Current config has debug enabled. Updating."),
               asst_tc([c2]), r2, asst("Updated config: debug disabled, workers set to 4.")], ["read_file", "write_file"]

def _multi_grep_read():
    b = rand_bond()
    d = rand_dir()
    c1 = tc("grep", {"pattern": "api_key", "path": d})
    p = rpath(d, ".env")
    r1 = tool_msg(c1["id"], {"matches": [{"file": p, "line": 3, "text": "API_KEY=sk-abc123"}], "count": 1})
    c2 = tc("read_file", {"path": p})
    r2 = tool_msg(c2["id"], {"content": "DATABASE_URL=postgres://localhost/db\nDEBUG=true\nAPI_KEY=sk-abc123\nSECRET=mysecret", "lines": 4})
    return b, [sys(b), usr(f"Find where api_key is set in {d} and show me the full file"),
               asst_tc([c1]), r1, asst(f"Found in {p} at line 3. Reading full file."),
               asst_tc([c2]), r2, asst(f"The API key is in {p}. Here's the full contents:\n\n```\nDATABASE_URL=postgres://localhost/db\nDEBUG=true\nAPI_KEY=sk-abc123\nSECRET=mysecret\n```")], ["grep", "read_file"]

def _multi_disk_large():
    b = rand_bond()
    c1 = tc("disk_space", {})
    r1 = tool_msg(c1["id"], {"total": "500GB", "used": "465GB", "free": "35GB", "percent_used": 93})
    c2 = tc("large_files", {"path": "/home/user", "limit": 5, "min_size": "100MB"})
    files = [{"path": "/home/user/Videos/recording.mp4", "size": "2.4GB"},
             {"path": "/home/user/Downloads/dataset.csv", "size": "800MB"}]
    r2 = tool_msg(c2["id"], {"files": files, "count": 2})
    return b, [sys(b), usr("I'm almost out of disk space. Help me figure out why."),
               asst_tc([c1]), r1, asst("93% full with only 35GB free. Scanning for large files."),
               asst_tc([c2]), r2, asst("Two big offenders:\n- recording.mp4 (2.4GB)\n- dataset.csv (800MB)\n\nTrash either to recover space.")], ["disk_space", "large_files"]

# ---------------------------------------------------------------------------
# Error scenarios (9% of 250 = ~23)
# ---------------------------------------------------------------------------
ERROR_SCENARIOS = [
    lambda: _err_not_found(),
    lambda: _err_permission_denied(),
    lambda: _err_disk_full(),
    lambda: _err_not_found_search(),
    lambda: _err_read_dir(),
    lambda: _err_rmdir_notempty(),
    lambda: _err_move_exists(),
    lambda: _err_decompress_corrupt(),
    lambda: _err_edit_no_match(),
    lambda: _err_trash_not_found(),
]

def _err_not_found():
    b = rand_bond()
    p = rpath(rand_dir(), "nonexistent.txt")
    c = tc("read_file", {"path": p})
    r = tool_msg(c["id"], {"error": "FileNotFound", "message": f"No such file: {p}"})
    resps = [f"That file doesn't exist: {p}", f"{p} not found.", f"No file at {p}. Check the path?"]
    return b, [sys(b), usr(f"Read {p}"), asst_tc([c]), r, asst(random.choice(resps))], ["read_file"], "error_handling"

def _err_permission_denied():
    b = rand_bond()
    p = "/etc/shadow"
    c = tc("read_file", {"path": p})
    r = tool_msg(c["id"], {"error": "PermissionDenied", "message": f"Permission denied: {p}"})
    return b, [sys(b), usr(f"Show me {p}"), asst_tc([c]), r, asst(f"Permission denied on {p}. That file requires root access.")], ["read_file"], "error_handling"

def _err_disk_full():
    b = rand_bond()
    p = rpath(rand_dir(), "large_output.bin")
    c = tc("write_file", {"path": p, "content": "data..."})
    r = tool_msg(c["id"], {"error": "DiskFull", "message": "No space left on device"})
    return b, [sys(b), usr(f"Write output to {p}"), asst_tc([c]), r, asst("Write failed -- disk is full. You'll need to free up space first.")], ["write_file"], "error_handling"

def _err_not_found_search():
    b = rand_bond()
    fname = "mystery_file.xyz"
    c = tc("search_files", {"name": fname, "path": "/home/user"})
    r = tool_msg(c["id"], {"matches": [], "count": 0})
    return b, [sys(b), usr(f"Find {fname}"), asst_tc([c]), r, asst(f"No files matching '{fname}' found anywhere under /home/user.")], ["search_files"], "error_handling"

def _err_read_dir():
    b = rand_bond()
    p = rand_dir()
    c = tc("read_file", {"path": p})
    r = tool_msg(c["id"], {"error": "IsDirectory", "message": f"{p} is a directory, not a file"})
    return b, [sys(b), usr(f"Read {p}"), asst_tc([c]), r, asst(f"That's a directory, not a file. Want me to list its contents instead?")], ["read_file"], "error_handling"

def _err_rmdir_notempty():
    b = rand_bond()
    p = rpath(rand_dir(), "important_data")
    c = tc("rmdir", {"path": p, "recursive": False})
    r = tool_msg(c["id"], {"error": "DirectoryNotEmpty", "message": f"{p} is not empty"})
    return b, [sys(b), usr(f"Remove {p}"), asst_tc([c]), r, asst(f"Can't remove {p} -- it's not empty. Use recursive deletion if you're sure, or clean it out first.")], ["rmdir"], "error_handling"

def _err_move_exists():
    b = rand_bond()
    fname = random.choice(DOC_FILES)
    src = rpath(rand_dir(), fname)
    dst = rpath(rand_dir(), fname)
    c = tc("move_file", {"source": src, "destination": dst})
    r = tool_msg(c["id"], {"error": "AlreadyExists", "message": f"Destination already exists: {dst}"})
    return b, [sys(b), usr(f"Move {src} to {dst}"), asst_tc([c]), r, asst(f"A file already exists at {dst}. Rename it or remove the existing one first?")], ["move_file"], "error_handling"

def _err_decompress_corrupt():
    b = rand_bond()
    p = rpath("/home/user/Downloads", "corrupted.zip")
    c = tc("decompress", {"source": p, "destination": "/home/user/extracted"})
    r = tool_msg(c["id"], {"error": "CorruptArchive", "message": "Archive is corrupt or incomplete"})
    return b, [sys(b), usr(f"Extract {p}"), asst_tc([c]), r, asst("That archive is corrupt or incomplete. You may need to re-download it.")], ["decompress"], "error_handling"

def _err_edit_no_match():
    b = rand_bond()
    p = rpath(rand_dir(), "config.yaml")
    c = tc("edit_file", {"path": p, "old_text": "nonexistent_line", "new_text": "replacement"})
    r = tool_msg(c["id"], {"error": "NoMatch", "message": "old_text not found in file"})
    return b, [sys(b), usr(f"Replace 'nonexistent_line' in {p}"), asst_tc([c]), r, asst("That text wasn't found in the file. Want me to read it so you can identify the correct string?")], ["edit_file"], "error_handling"

def _err_trash_not_found():
    b = rand_bond()
    tid = "trash_9999"
    c = tc("restore_trash", {"trash_id": tid})
    r = tool_msg(c["id"], {"error": "NotFound", "message": f"No trash item with id {tid}"})
    return b, [sys(b), usr("Restore that file I just deleted"), asst_tc([c]), r, asst("Couldn't find that item in trash. Want me to list what's in the trash?")], ["restore_trash"], "error_handling"

# ---------------------------------------------------------------------------
# Main generation
# ---------------------------------------------------------------------------
SINGLE_GENERATORS = [
    (gen_list_files, 10), (gen_read_file, 10), (gen_write_file, 8),
    (gen_search_files, 8), (gen_edit_file, 8), (gen_grep, 8),
    (gen_glob, 7), (gen_mkdir, 5), (gen_rmdir, 4),
    (gen_move_file, 5), (gen_copy_file, 5), (gen_file_info, 5),
    (gen_file_hash, 4), (gen_compress, 4), (gen_decompress, 4),
    (gen_watch_file, 3), (gen_trash, 5), (gen_restore_trash, 3),
    (gen_list_trash, 3), (gen_disk_space, 4), (gen_mount_info, 3),
    (gen_symlink, 3), (gen_permissions, 4), (gen_ownership, 3),
    (gen_find_duplicates, 4), (gen_recent_files, 4), (gen_large_files, 4),
    (gen_file_type, 3), (gen_count_lines, 4), (gen_diff_files, 3),
]  # total single = 160

def main():
    examples = []

    # Single-tool examples
    for gen_fn, count in SINGLE_GENERATORS:
        for i in range(count):
            b, msgs, tools = gen_fn(i)
            examples.append(line(msgs, b, tools))

    # Multi-tool examples (45)
    for i in range(45):
        fn = MULTI_SCENARIOS[i % len(MULTI_SCENARIOS)]
        b, msgs, tools = fn()
        examples.append(line(msgs, b, tools, "multi_tool"))

    # Error examples (23)
    for i in range(23):
        fn = ERROR_SCENARIOS[i % len(ERROR_SCENARIOS)]
        result = fn()
        b, msgs, tools = result[0], result[1], result[2]
        scenario = result[3] if len(result) > 3 else "error_handling"
        examples.append(line(msgs, b, tools, scenario))

    # Pad to exactly 250 with extra single-tool
    while len(examples) < 250:
        gen_fn, _ = random.choice(SINGLE_GENERATORS)
        b, msgs, tools = gen_fn()
        examples.append(line(msgs, b, tools))

    # Trim if over
    examples = examples[:250]

    random.shuffle(examples)

    out_path = OUT_DIR / "batch_tools_02_files.jsonl"
    with open(out_path, "w", encoding="utf-8") as f:
        for ex in examples:
            f.write(ex + "\n")

    # Stats
    tools_used = {}
    bonds = {}
    scenarios = {}
    for ex in examples:
        data = json.loads(ex)
        meta = data["metadata"]
        bonds[meta["bond_stage"]] = bonds.get(meta["bond_stage"], 0) + 1
        scenarios[meta["scenario_type"]] = scenarios.get(meta["scenario_type"], 0) + 1
        for t in meta["tools_used"]:
            tools_used[t] = tools_used.get(t, 0) + 1

    print(f"Generated {len(examples)} examples -> {out_path}")
    print(f"\nBond stages: {bonds}")
    print(f"Scenarios: {scenarios}")
    print(f"\nTool coverage ({len(tools_used)}/30):")
    for t in sorted(tools_used.keys()):
        print(f"  {t}: {tools_used[t]}")

if __name__ == "__main__":
    main()
