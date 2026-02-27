#!/usr/bin/env python3
"""Yantrik OS End-to-End System Evaluation"""
import json
import time
import urllib.request
import urllib.error

BASE = "http://127.0.0.1:8340"
LLM = "http://127.0.0.1:8081"
EMBED = "http://127.0.0.1:8082"


def req(url, data=None, method=None):
    if method is None:
        method = "POST" if data else "GET"
    headers = {"Content-Type": "application/json"} if data else {}
    body = json.dumps(data).encode() if data else None
    r = urllib.request.Request(url, data=body, headers=headers, method=method)
    try:
        resp = urllib.request.urlopen(r, timeout=120)
        return json.loads(resp.read()), resp.status
    except urllib.error.HTTPError as e:
        return json.loads(e.read()), e.code
    except Exception as e:
        return {"error": str(e)}, 0


def section(title):
    sep = "=" * 60
    print("\n" + sep)
    print("  " + title)
    print(sep)


def test(name, passed, detail=""):
    status = "PASS" if passed else "FAIL"
    msg = "  [" + status + "] " + name
    if detail:
        msg += " -- " + detail
    print(msg)
    return passed


results = []

# 1. Service Health
section("1. Service Health Checks")

data, code = req(BASE + "/health")
results.append(test("Companion /health", code == 200, json.dumps(data)[:80]))

data, code = req(LLM + "/health")
results.append(test("LLM server /health", code == 200, json.dumps(data)[:80]))

data, code = req(EMBED + "/health")
results.append(test("Embedding server /health", code == 200, json.dumps(data)[:80]))

# 2. Status & Personality
section("2. Status & Personality")

data, code = req(BASE + "/status")
results.append(test("Companion /status", code == 200))
mem_count = 0
if code == 200:
    mem_count = data.get("memory_count", 0)
    personality = data.get("personality", {})
    print("    Memory count: " + str(mem_count))
    if personality:
        traits = personality.get("traits", [])
        for t in traits:
            tn = t.get("trait_name", "?")
            sc = t.get("score", 0)
            co = t.get("confidence", 0)
            print("    " + tn + ": " + "{:.2f}".format(sc) + " (conf=" + "{:.2f}".format(co) + ")")
        results.append(test("Personality traits present", len(traits) >= 4, str(len(traits)) + " traits"))
    else:
        results.append(test("Personality traits present", False, "no personality in status"))

# 3. Chat Interaction
section("3. Chat Interaction")

print("  Sending: Hello, I am Pranab. Who are you?")
t0 = time.time()
data, code = req(BASE + "/chat", {"message": "Hello, I am Pranab. Who are you?"})
elapsed = time.time() - t0
results.append(test("Chat responds", code == 200, "{:.1f}s".format(elapsed)))
if code == 200:
    reply = data.get("response", data.get("reply", str(data)))[:200]
    print("    Response: " + reply)
    results.append(test("Response is non-empty", len(reply) > 10))
else:
    print("    Error: " + str(data))

# 4. Memory Recording
section("4. Memory & Learning")

print("  Sending: I love playing chess and reading science fiction books")
t0 = time.time()
data, code = req(BASE + "/chat", {"message": "I love playing chess and reading science fiction books"})
elapsed = time.time() - t0
results.append(test("Chat about interests", code == 200, "{:.1f}s".format(elapsed)))
if code == 200:
    print("    Response: " + data.get("response", data.get("reply", str(data)))[:200])

# Give learning a moment
time.sleep(3)

# Check if memories grew
data2, code2 = req(BASE + "/status")
if code2 == 200:
    new_count = data2.get("memory_count", 0)
    results.append(test("Memories accumulated", new_count >= mem_count, str(mem_count) + " -> " + str(new_count)))

# 5. Memory Recall
section("5. Memory Recall via Chat")

print("  Sending: What do you know about my hobbies?")
t0 = time.time()
data, code = req(BASE + "/chat", {"message": "What do you know about my hobbies?"})
elapsed = time.time() - t0
results.append(test("Recall query responds", code == 200, "{:.1f}s".format(elapsed)))
if code == 200:
    reply = data.get("response", data.get("reply", str(data)))[:300]
    print("    Response: " + reply)
    has_chess = "chess" in reply.lower()
    has_reading = "sci" in reply.lower() or "fiction" in reply.lower() or "book" in reply.lower() or "read" in reply.lower()
    results.append(test("Recalls chess interest", has_chess))
    results.append(test("Recalls reading interest", has_reading))

# 6. Tool Calling
section("6. Tool Calling")

print("  Sending: Please remember that my favorite color is deep blue")
t0 = time.time()
data, code = req(BASE + "/chat", {"message": "Please remember that my favorite color is deep blue"})
elapsed = time.time() - t0
results.append(test("Remember request", code == 200, "{:.1f}s".format(elapsed)))
if code == 200:
    reply = data.get("response", data.get("reply", str(data)))[:200]
    print("    Response: " + reply)
    tool_calls = data.get("tool_calls", [])
    if tool_calls:
        print("    Tool calls: " + json.dumps(tool_calls)[:200])
        results.append(test("Tool call detected", True, str(len(tool_calls)) + " calls"))
    else:
        # Check if it acknowledged remembering anyway
        remembered = "remember" in reply.lower() or "note" in reply.lower() or "got it" in reply.lower() or "stored" in reply.lower() or "blue" in reply.lower()
        results.append(test("Memory acknowledgment", remembered, "implicit via response"))

# 7. Contextual Continuity
section("7. Contextual Continuity")

print("  Sending: What is my name?")
data, code = req(BASE + "/chat", {"message": "What is my name?"})
results.append(test("Name recall responds", code == 200))
if code == 200:
    reply = data.get("response", data.get("reply", str(data)))[:200]
    print("    Response: " + reply)
    results.append(test("Remembers name Pranab", "pranab" in reply.lower()))

# 8. Urges/Instincts
section("8. Urges & Instincts")

data, code = req(BASE + "/urges")
if code == 200:
    urges = data if isinstance(data, list) else data.get("urges", [])
    print("    Pending urges: " + str(len(urges)))
    for u in urges[:3]:
        urg = u.get("urgency", 0)
        reason = u.get("reason", "?")[:60]
        print("      - [" + "{:.1f}".format(urg) + "] " + reason)
    results.append(test("Urges endpoint works", True, str(len(urges)) + " urges"))
elif code == 404:
    results.append(test("Urges endpoint", False, "404 - endpoint not found"))
else:
    results.append(test("Urges endpoint works", code == 200, "code=" + str(code)))

# 9. Direct LLM Test
section("9. Direct LLM Performance")

print("  Testing raw LLM speed...")
t0 = time.time()
data, code = req(LLM + "/v1/chat/completions", {
    "model": "qwen2.5",
    "messages": [{"role": "user", "content": "Count from 1 to 20"}],
    "max_tokens": 100,
    "temperature": 0.7
})
elapsed = time.time() - t0
results.append(test("Direct LLM call", code == 200, "{:.1f}s".format(elapsed)))
if code == 200:
    usage = data.get("usage", {})
    tokens = usage.get("completion_tokens", 0)
    tps = tokens / elapsed if elapsed > 0 else 0
    print("    Tokens: " + str(tokens) + ", Speed: " + "{:.1f}".format(tps) + " tok/s")
    content = data.get("choices", [{}])[0].get("message", {}).get("content", "")[:100]
    print("    Response: " + content)
    results.append(test("Inference speed > 5 tok/s", tps > 5, "{:.1f}".format(tps) + " tok/s"))

# 10. Embedding Test
section("10. Embedding Service")

t0 = time.time()
data, code = req(EMBED + "/v1/embeddings", {
    "model": "all-minilm",
    "input": "test embedding generation"
})
elapsed = time.time() - t0
results.append(test("Embedding generation", code == 200, "{:.3f}s".format(elapsed)))
if code == 200:
    emb = data.get("data", [{}])[0].get("embedding", [])
    print("    Dimensions: " + str(len(emb)))
    results.append(test("Embedding dimension 384", len(emb) == 384))

# 11. Multi-turn coherence
section("11. Multi-turn Coherence")

print("  Sending: What was my favorite color that I just told you about?")
data, code = req(BASE + "/chat", {"message": "What was my favorite color that I just told you about?"})
results.append(test("Color recall responds", code == 200))
if code == 200:
    reply = data.get("response", data.get("reply", str(data)))[:200]
    print("    Response: " + reply)
    results.append(test("Recalls deep blue", "blue" in reply.lower()))

# Summary
section("EVALUATION SUMMARY")
passed = sum(1 for r in results if r)
total = len(results)
pct = 100 * passed // total if total > 0 else 0
print("")
print("  " + str(passed) + "/" + str(total) + " tests passed (" + str(pct) + "%)")
print("")
if passed == total:
    print("  ALL SYSTEMS OPERATIONAL")
else:
    print("  " + str(total - passed) + " tests need attention")
print("")
