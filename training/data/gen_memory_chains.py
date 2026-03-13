#!/usr/bin/env python3
"""Generate 200 JSONL training examples for multi-tool memory operations,
memory hygiene, forget flows, and complex memory chains."""

import json
import random
import os

random.seed(2026)

OUT = os.path.join(os.path.dirname(__file__), "batch_memory_03_chains.jsonl")

NAMES = ["Alex", "Sam", "Maya", "Jordan", "Priya", "Leo", "Kai", "Noor",
         "Tess", "Ravi", "Zoe", "Finn", "Anya", "Cole", "Devi"]

BOND_STAGES = ["stranger", "acquaintance", "trusted", "deep"]
BOND_WEIGHTS = [0.1, 0.25, 0.35, 0.3]

MEM_COUNTS = list(range(30, 500))
TIMES = [
    "2026-03-13 08:15:00", "2026-03-13 10:42:00", "2026-03-13 14:30:00",
    "2026-03-13 17:05:00", "2026-03-13 20:22:00", "2026-03-13 23:10:00",
    "2026-03-12 09:00:00", "2026-03-11 16:45:00",
]

_call_counter = 0

def next_call_id():
    global _call_counter
    _call_counter += 1
    return f"call_{_call_counter:04d}"

def make_system(name, bond, mem_count, time):
    bond_desc = {
        "stranger": f"Bond stage: STRANGER. Be helpful and polite but reserved with {name}. Use full sentences.",
        "acquaintance": f"Bond stage: ACQUAINTANCE. You're friendly with {name}. You remember some past interactions.",
        "trusted": f"Bond stage: TRUSTED. You're casual and direct with {name}. You share opinions freely.",
        "deep": f"Bond stage: DEEP. You know {name} well. You're warm, direct, sometimes playful.",
    }[bond]
    return (
        f"You are Yantrik, {name}'s personal companion. {bond_desc} "
        f"You have persistent memory with {mem_count} stored memories. "
        f"You can chain multiple memory operations to serve the user. "
        f"Current time: {time}. "
        f"Tools available: recall, remember, save_user_fact, forget_topic, forget_memory, "
        f"memory_stats, review_memories, update_memory, relate_entities, introspect, "
        f"search_fix_history, search_by_timeframe, purge_system_noise. "
        f"Be concise. No filler phrases. No emoji."
    )

def tool_msg(call_id, name, arguments):
    """Build an assistant message with one tool call."""
    return {
        "role": "assistant",
        "content": None,
        "tool_calls": [{
            "id": call_id,
            "type": "function",
            "function": {
                "name": name,
                "arguments": json.dumps(arguments),
            }
        }]
    }

def tool_result(call_id, result):
    return {"role": "tool", "content": json.dumps(result), "tool_call_id": call_id}

def pick_context():
    name = random.choice(NAMES)
    bond = random.choices(BOND_STAGES, BOND_WEIGHTS)[0]
    mem_count = random.choice(MEM_COUNTS)
    time = random.choice(TIMES)
    return name, bond, mem_count, time

def make_example(user_msg, tool_chain, final_response, scenario_type):
    """Build one JSONL-ready dict.
    tool_chain: list of (tool_name, tool_args, tool_result_dict) tuples
    """
    name, bond, mem_count, time = pick_context()
    messages = [
        {"role": "system", "content": make_system(name, bond, mem_count, time)},
        {"role": "user", "content": user_msg.replace("{name}", name)},
    ]
    for tname, targs, tresult in tool_chain:
        cid = next_call_id()
        messages.append(tool_msg(cid, tname, targs))
        messages.append(tool_result(cid, tresult))
    messages.append({"role": "assistant", "content": final_response.replace("{name}", name)})
    return {
        "conversations": messages,
        "metadata": {"bond_stage": bond, "scenario_type": scenario_type},
    }

# ---------------------------------------------------------------------------
# CATEGORY 1: Recall -> Remember chains (50 examples)
# ---------------------------------------------------------------------------
RECALL_REMEMBER = [
    # (user, recall_args, recall_result, remember_args, remember_result, final)
    (
        "I got promoted! Did you know I was up for it?",
        {"query": "promotion job career"},
        {"memories": [{"id": "mem_r01", "text": "User mentioned hoping for a promotion at work in January.", "score": 0.91, "created": "2026-01-18"}], "total": 1},
        {"text": "User got promoted at work!", "importance": 0.9, "memory_type": "episodic", "domain": "work"},
        {"status": "stored", "rid": "mem_r02"},
        "You did it! You've been working toward this since January. Congratulations."
    ),
    (
        "We ended up going with React after all",
        {"query": "framework decision React Vue"},
        {"memories": [{"id": "mem_r03", "text": "User's team debating between React and Vue for frontend rewrite.", "score": 0.88, "created": "2026-02-10"}], "total": 1},
        {"text": "Team chose React over Vue for the frontend rewrite.", "importance": 0.6, "memory_type": "semantic", "domain": "work"},
        {"status": "stored", "rid": "mem_r04"},
        "Ha, I remember you going back and forth on that. React it is."
    ),
    (
        "I finally finished my thesis!",
        {"query": "thesis dissertation writing"},
        {"memories": [{"id": "mem_r05", "text": "User has been stressed about finishing their thesis for months.", "score": 0.93, "created": "2026-01-05"}], "total": 1},
        {"text": "User completed their thesis.", "importance": 0.95, "memory_type": "episodic", "domain": "education"},
        {"status": "stored", "rid": "mem_r06"},
        "Finally! You've been grinding on that since at least January. Huge milestone."
    ),
    (
        "I switched to a standing desk",
        {"query": "desk workspace ergonomics"},
        {"memories": [{"id": "mem_r07", "text": "User complained about back pain from sitting all day.", "score": 0.85, "created": "2026-02-20"}], "total": 1},
        {"text": "User switched to a standing desk.", "importance": 0.5, "memory_type": "episodic", "domain": "health"},
        {"status": "stored", "rid": "mem_r08"},
        "Good move. You were dealing with back pain from the old setup."
    ),
    (
        "I broke up with Jamie",
        {"query": "Jamie relationship partner"},
        {"memories": [{"id": "mem_r09", "text": "User is dating someone named Jamie. Been together about 8 months.", "score": 0.92, "created": "2026-01-12"}], "total": 1},
        {"text": "User broke up with Jamie.", "importance": 0.85, "memory_type": "episodic", "domain": "personal"},
        {"status": "stored", "rid": "mem_r10"},
        "I'm sorry. That's a big change after 8 months. I'm here if you want to talk."
    ),
    (
        "We adopted a cat! Her name is Luna",
        {"query": "pet cat adoption"},
        {"memories": [{"id": "mem_r11", "text": "User mentioned wanting to adopt a cat.", "score": 0.87, "created": "2026-02-28"}], "total": 1},
        {"text": "User adopted a cat named Luna.", "importance": 0.7, "memory_type": "episodic", "domain": "personal"},
        {"status": "stored", "rid": "mem_r12"},
        "Luna! You've been talking about getting a cat. Glad you went through with it."
    ),
    (
        "I passed the AWS certification exam",
        {"query": "AWS certification exam study"},
        {"memories": [{"id": "mem_r13", "text": "User studying for AWS Solutions Architect certification.", "score": 0.90, "created": "2026-02-05"}], "total": 1},
        {"text": "User passed the AWS Solutions Architect certification exam.", "importance": 0.85, "memory_type": "episodic", "domain": "work"},
        {"status": "stored", "rid": "mem_r14"},
        "Nice work. All that studying paid off."
    ),
    (
        "I moved to the new apartment finally",
        {"query": "apartment moving new place"},
        {"memories": [{"id": "mem_r15", "text": "User is planning to move to a new apartment in March.", "score": 0.89, "created": "2026-02-18"}], "total": 1},
        {"text": "User moved to the new apartment in March 2026.", "importance": 0.7, "memory_type": "episodic", "domain": "personal"},
        {"status": "stored", "rid": "mem_r16"},
        "You made it happen right on schedule. How's the new place?"
    ),
    (
        "I started taking blood pressure medication",
        {"query": "blood pressure health medication"},
        {"memories": [{"id": "mem_r17", "text": "User mentioned high blood pressure at last doctor visit.", "score": 0.91, "created": "2026-02-25"}], "total": 1},
        {"text": "User started blood pressure medication.", "importance": 0.8, "memory_type": "episodic", "domain": "health"},
        {"status": "stored", "rid": "mem_r18"},
        "Noted. Makes sense after what the doctor said last month. I can remind you to take it if you want."
    ),
    (
        "Our team shipped the v2 release yesterday",
        {"query": "v2 release shipping product"},
        {"memories": [{"id": "mem_r19", "text": "User's team has been crunching on the v2 release for weeks.", "score": 0.88, "created": "2026-03-01"}], "total": 1},
        {"text": "Team shipped the v2 release on March 12, 2026.", "importance": 0.75, "memory_type": "episodic", "domain": "work"},
        {"status": "stored", "rid": "mem_r20"},
        "After all that crunch. Bet that feels good."
    ),
    (
        "I decided to learn piano",
        {"query": "piano music learning hobby"},
        {"memories": [], "total": 0},
        {"text": "User decided to start learning piano.", "importance": 0.6, "memory_type": "episodic", "domain": "hobbies"},
        {"status": "stored", "rid": "mem_r21"},
        "That's new. No prior mentions of piano from you. Starting from scratch?"
    ),
    (
        "My mom got out of the hospital",
        {"query": "mom mother hospital health"},
        {"memories": [{"id": "mem_r22", "text": "User's mother was hospitalized for surgery last week.", "score": 0.94, "created": "2026-03-06"}], "total": 1},
        {"text": "User's mother discharged from hospital, recovering.", "importance": 0.85, "memory_type": "episodic", "domain": "family"},
        {"status": "stored", "rid": "mem_r23"},
        "That's a relief. Hope her recovery goes smoothly."
    ),
    (
        "I switched from VS Code to Neovim full time",
        {"query": "editor VS Code Neovim IDE"},
        {"memories": [{"id": "mem_r24", "text": "User uses VS Code as primary editor.", "score": 0.87, "created": "2026-01-20"}], "total": 1},
        {"text": "User switched primary editor from VS Code to Neovim.", "importance": 0.6, "memory_type": "semantic", "domain": "work"},
        {"status": "stored", "rid": "mem_r25"},
        "Big switch. I had you down as a VS Code user. Updated."
    ),
    (
        "We named the baby Oliver",
        {"query": "baby name child expecting"},
        {"memories": [{"id": "mem_r26", "text": "User and partner expecting a baby, due in March.", "score": 0.95, "created": "2026-01-30"}], "total": 1},
        {"text": "User's baby born, named Oliver.", "importance": 0.95, "memory_type": "episodic", "domain": "family"},
        {"status": "stored", "rid": "mem_r27"},
        "Oliver. Congratulations to you both. That's a big moment."
    ),
    (
        "I quit smoking last Tuesday",
        {"query": "smoking cigarettes quitting"},
        {"memories": [{"id": "mem_r28", "text": "User smokes and has mentioned wanting to quit.", "score": 0.86, "created": "2026-02-14"}], "total": 1},
        {"text": "User quit smoking on March 4, 2026.", "importance": 0.9, "memory_type": "episodic", "domain": "health"},
        {"status": "stored", "rid": "mem_r29"},
        "That's huge. You talked about wanting to quit. One week in already."
    ),
    (
        "I got a new car, it's a Tesla Model 3",
        {"query": "car vehicle driving"},
        {"memories": [{"id": "mem_r30", "text": "User drives a 2019 Honda Civic.", "score": 0.84, "created": "2026-01-08"}], "total": 1},
        {"text": "User bought a Tesla Model 3, replacing 2019 Honda Civic.", "importance": 0.65, "memory_type": "episodic", "domain": "personal"},
        {"status": "stored", "rid": "mem_r31"},
        "Big upgrade from the Civic. How's the Tesla treating you?"
    ),
    (
        "I started a side project, a recipe app",
        {"query": "side project app development"},
        {"memories": [{"id": "mem_r32", "text": "User mentioned wanting to build something on the side.", "score": 0.79, "created": "2026-02-22"}], "total": 1},
        {"text": "User started a side project: a recipe app.", "importance": 0.6, "memory_type": "episodic", "domain": "work"},
        {"status": "stored", "rid": "mem_r33"},
        "You mentioned wanting a side project. A recipe app sounds fun. What stack?"
    ),
    (
        "I changed my work schedule to four days a week",
        {"query": "work schedule hours days"},
        {"memories": [{"id": "mem_r34", "text": "User works Monday to Friday, 9 to 5.", "score": 0.85, "created": "2026-01-15"}], "total": 1},
        {"text": "User switched to a 4-day work week.", "importance": 0.7, "memory_type": "semantic", "domain": "work"},
        {"status": "stored", "rid": "mem_r35"},
        "Updated. You were on the standard five days before. Which day off?"
    ),
    (
        "My sister got engaged!",
        {"query": "sister family engagement"},
        {"memories": [{"id": "mem_r36", "text": "User's sister is dating someone named Marcus.", "score": 0.82, "created": "2026-02-03"}], "total": 1},
        {"text": "User's sister got engaged.", "importance": 0.75, "memory_type": "episodic", "domain": "family"},
        {"status": "stored", "rid": "mem_r37"},
        "To Marcus? That's exciting news. Happy for them."
    ),
    (
        "I finally hit a 200lb deadlift",
        {"query": "deadlift gym weightlifting fitness"},
        {"memories": [{"id": "mem_r38", "text": "User working toward a 200lb deadlift goal.", "score": 0.90, "created": "2026-02-12"}], "total": 1},
        {"text": "User hit 200lb deadlift PR.", "importance": 0.7, "memory_type": "episodic", "domain": "health"},
        {"status": "stored", "rid": "mem_r39"},
        "There it is. You've been chasing that number. Solid work."
    ),
    (
        "I deleted my Twitter account",
        {"query": "Twitter social media account"},
        {"memories": [{"id": "mem_r40", "text": "User has been considering leaving Twitter for a while.", "score": 0.83, "created": "2026-02-08"}], "total": 1},
        {"text": "User deleted their Twitter account.", "importance": 0.5, "memory_type": "episodic", "domain": "personal"},
        {"status": "stored", "rid": "mem_r41"},
        "You've been on the fence about that. No looking back now."
    ),
    (
        "I accepted the job offer from Stripe",
        {"query": "job offer interview Stripe"},
        {"memories": [{"id": "mem_r42", "text": "User had a final round interview at Stripe last week.", "score": 0.93, "created": "2026-03-05"}], "total": 1},
        {"text": "User accepted a job offer from Stripe.", "importance": 0.95, "memory_type": "episodic", "domain": "work"},
        {"status": "stored", "rid": "mem_r43"},
        "That's a big deal. The interview went well then. When do you start?"
    ),
    (
        "I finished reading Dune finally",
        {"query": "Dune book reading"},
        {"memories": [{"id": "mem_r44", "text": "User has been reading Dune for several weeks.", "score": 0.86, "created": "2026-02-15"}], "total": 1},
        {"text": "User finished reading Dune.", "importance": 0.4, "memory_type": "episodic", "domain": "hobbies"},
        {"status": "stored", "rid": "mem_r45"},
        "Only took you a month. What did you think of it?"
    ),
    (
        "I set up the home server with Proxmox",
        {"query": "home server Proxmox setup homelab"},
        {"memories": [{"id": "mem_r46", "text": "User planning to set up a home server for containers.", "score": 0.88, "created": "2026-02-27"}], "total": 1},
        {"text": "User set up home server running Proxmox.", "importance": 0.6, "memory_type": "semantic", "domain": "tech"},
        {"status": "stored", "rid": "mem_r47"},
        "Nice. You were planning that. What are you running on it?"
    ),
    (
        "My company got acquired by Google",
        {"query": "company acquisition"},
        {"memories": [{"id": "mem_r48", "text": "User works at a mid-size startup.", "score": 0.80, "created": "2026-01-22"}], "total": 1},
        {"text": "User's company was acquired by Google.", "importance": 0.9, "memory_type": "episodic", "domain": "work"},
        {"status": "stored", "rid": "mem_r49"},
        "That's massive. How's the team feeling about it?"
    ),
    (
        "I signed up for a marathon in October",
        {"query": "marathon running race"},
        {"memories": [{"id": "mem_r50", "text": "User runs 3-4 times a week, about 5K each time.", "score": 0.83, "created": "2026-02-01"}], "total": 1},
        {"text": "User signed up for a marathon in October 2026.", "importance": 0.7, "memory_type": "episodic", "domain": "health"},
        {"status": "stored", "rid": "mem_r51"},
        "From 5K runs to a full marathon. That's a jump. You've got 7 months to train."
    ),
    (
        "I started therapy",
        {"query": "therapy mental health counseling"},
        {"memories": [{"id": "mem_r52", "text": "User mentioned feeling burned out and overwhelmed lately.", "score": 0.88, "created": "2026-03-02"}], "total": 1},
        {"text": "User started therapy.", "importance": 0.8, "memory_type": "episodic", "domain": "health"},
        {"status": "stored", "rid": "mem_r53"},
        "Good step, especially with how you've been feeling. I hope it helps."
    ),
    (
        "We got a new CTO, her name is Lisa Park",
        {"query": "CTO leadership company"},
        {"memories": [{"id": "mem_r54", "text": "User's company CTO resigned last month.", "score": 0.85, "created": "2026-02-20"}], "total": 1},
        {"text": "New CTO at user's company: Lisa Park.", "importance": 0.65, "memory_type": "semantic", "domain": "work"},
        {"status": "stored", "rid": "mem_r55"},
        "Good to have someone in that seat again. Know anything about her background?"
    ),
    (
        "I paid off my student loans today!",
        {"query": "student loans debt payments"},
        {"memories": [{"id": "mem_r56", "text": "User has been aggressively paying down student loan debt.", "score": 0.91, "created": "2026-01-25"}], "total": 1},
        {"text": "User paid off all student loans as of March 2026.", "importance": 0.9, "memory_type": "episodic", "domain": "finance"},
        {"status": "stored", "rid": "mem_r57"},
        "That's freedom. You've been chipping away at those for a while."
    ),
    (
        "I switched my phone to a Pixel 9",
        {"query": "phone device mobile"},
        {"memories": [{"id": "mem_r58", "text": "User uses an iPhone 14.", "score": 0.84, "created": "2026-01-10"}], "total": 1},
        {"text": "User switched from iPhone 14 to Pixel 9.", "importance": 0.45, "memory_type": "semantic", "domain": "tech"},
        {"status": "stored", "rid": "mem_r59"},
        "From iPhone to Pixel. That's a full ecosystem switch. How's it feel?"
    ),
    (
        "My best friend moved to Berlin",
        {"query": "best friend moving"},
        {"memories": [{"id": "mem_r60", "text": "User's best friend Alex lives nearby.", "score": 0.80, "created": "2026-01-18"}], "total": 1},
        {"text": "User's best friend moved to Berlin.", "importance": 0.7, "memory_type": "episodic", "domain": "personal"},
        {"status": "stored", "rid": "mem_r61"},
        "That's a big distance now. You two were close by before."
    ),
    (
        "I got my driver's license renewed",
        {"query": "driver license renewal"},
        {"memories": [{"id": "mem_r62", "text": "User's driver's license expiring soon, needs to renew.", "score": 0.87, "created": "2026-03-01"}], "total": 1},
        {"text": "User renewed driver's license in March 2026.", "importance": 0.4, "memory_type": "episodic", "domain": "personal"},
        {"status": "stored", "rid": "mem_r63"},
        "One less thing to worry about. You flagged that one just in time."
    ),
    (
        "I started intermittent fasting",
        {"query": "fasting diet eating schedule"},
        {"memories": [{"id": "mem_r64", "text": "User eats three meals a day, snacks a lot.", "score": 0.78, "created": "2026-02-05"}], "total": 1},
        {"text": "User started intermittent fasting.", "importance": 0.6, "memory_type": "episodic", "domain": "health"},
        {"status": "stored", "rid": "mem_r65"},
        "Big change from the three meals plus snacks routine. What window are you doing?"
    ),
    (
        "Our startup just raised Series A",
        {"query": "startup funding fundraising"},
        {"memories": [{"id": "mem_r66", "text": "User co-founded a startup, currently in seed stage.", "score": 0.90, "created": "2026-02-10"}], "total": 1},
        {"text": "User's startup raised Series A funding.", "importance": 0.85, "memory_type": "episodic", "domain": "work"},
        {"status": "stored", "rid": "mem_r67"},
        "From seed to Series A. That's real momentum. How much did you raise?"
    ),
    (
        "I cancelled my gym membership and bought a home gym",
        {"query": "gym membership fitness workout"},
        {"memories": [{"id": "mem_r68", "text": "User goes to the gym 4 times a week.", "score": 0.86, "created": "2026-01-28"}], "total": 1},
        {"text": "User cancelled gym membership, set up home gym instead.", "importance": 0.55, "memory_type": "semantic", "domain": "health"},
        {"status": "stored", "rid": "mem_r69"},
        "No more commute to the gym. What equipment did you get?"
    ),
    (
        "I got diagnosed with ADHD",
        {"query": "ADHD diagnosis mental health"},
        {"memories": [{"id": "mem_r70", "text": "User has trouble focusing and staying organized.", "score": 0.82, "created": "2026-02-16"}], "total": 1},
        {"text": "User diagnosed with ADHD.", "importance": 0.85, "memory_type": "episodic", "domain": "health"},
        {"status": "stored", "rid": "mem_r71"},
        "That explains a lot of what you've described. Are they recommending treatment?"
    ),
    (
        "I proposed to my girlfriend and she said yes!",
        {"query": "girlfriend relationship proposal"},
        {"memories": [{"id": "mem_r72", "text": "User has been dating their girlfriend for two years.", "score": 0.92, "created": "2026-01-14"}], "total": 1},
        {"text": "User got engaged! Proposed and she said yes.", "importance": 0.95, "memory_type": "episodic", "domain": "personal"},
        {"status": "stored", "rid": "mem_r73"},
        "Two years together and now engaged. That's wonderful. Congratulations."
    ),
    (
        "We migrated the whole backend to Kubernetes",
        {"query": "Kubernetes migration backend infrastructure"},
        {"memories": [{"id": "mem_r74", "text": "User's team planning a Kubernetes migration.", "score": 0.89, "created": "2026-02-23"}], "total": 1},
        {"text": "Team completed Kubernetes migration for the entire backend.", "importance": 0.7, "memory_type": "semantic", "domain": "work"},
        {"status": "stored", "rid": "mem_r75"},
        "That was a big undertaking. How's it running?"
    ),
    (
        "I started volunteering at the animal shelter",
        {"query": "volunteering community service"},
        {"memories": [], "total": 0},
        {"text": "User started volunteering at a local animal shelter.", "importance": 0.55, "memory_type": "episodic", "domain": "personal"},
        {"status": "stored", "rid": "mem_r76"},
        "That's great. First I'm hearing of it. How often are you going?"
    ),
    (
        "My landlord is raising the rent by 20%",
        {"query": "rent apartment landlord housing"},
        {"memories": [{"id": "mem_r77", "text": "User pays $1800/month in rent.", "score": 0.85, "created": "2026-01-20"}], "total": 1},
        {"text": "Landlord raising rent by 20%, from $1800 to ~$2160/month.", "importance": 0.75, "memory_type": "episodic", "domain": "finance"},
        {"status": "stored", "rid": "mem_r78"},
        "From $1800 to over $2100. That's steep. Are you thinking about moving?"
    ),
    (
        "I switched to Linux on my main machine",
        {"query": "operating system Linux desktop"},
        {"memories": [{"id": "mem_r79", "text": "User runs Windows 11 on main desktop.", "score": 0.86, "created": "2026-01-12"}], "total": 1},
        {"text": "User switched main desktop from Windows 11 to Linux.", "importance": 0.6, "memory_type": "semantic", "domain": "tech"},
        {"status": "stored", "rid": "mem_r80"},
        "Full switch from Windows. Which distro did you go with?"
    ),
    (
        "I got my first pull request merged at the new job",
        {"query": "new job first contribution"},
        {"memories": [{"id": "mem_r81", "text": "User started a new job recently.", "score": 0.88, "created": "2026-03-04"}], "total": 1},
        {"text": "User merged their first PR at the new job.", "importance": 0.65, "memory_type": "episodic", "domain": "work"},
        {"status": "stored", "rid": "mem_r82"},
        "First PR already. You're settling in fast."
    ),
    (
        "I stopped drinking coffee",
        {"query": "coffee caffeine drinking habits"},
        {"memories": [{"id": "mem_r83", "text": "User drinks 3 cups of coffee daily.", "score": 0.84, "created": "2026-02-01"}], "total": 1},
        {"text": "User quit coffee entirely.", "importance": 0.55, "memory_type": "episodic", "domain": "health"},
        {"status": "stored", "rid": "mem_r84"},
        "From three cups a day to zero. How are the withdrawal headaches?"
    ),
    (
        "I enrolled in a master's program at Stanford",
        {"query": "education degree master's program"},
        {"memories": [{"id": "mem_r85", "text": "User considering going back to school for a master's degree.", "score": 0.87, "created": "2026-02-18"}], "total": 1},
        {"text": "User enrolled in master's program at Stanford.", "importance": 0.85, "memory_type": "episodic", "domain": "education"},
        {"status": "stored", "rid": "mem_r86"},
        "Stanford. You were weighing your options, and you landed well."
    ),
    (
        "My dog Biscuit passed away",
        {"query": "Biscuit dog pet"},
        {"memories": [{"id": "mem_r87", "text": "User's dog is named Biscuit.", "score": 0.95, "created": "2026-01-05"}], "total": 1},
        {"text": "User's dog Biscuit passed away.", "importance": 0.9, "memory_type": "episodic", "domain": "personal"},
        {"status": "stored", "rid": "mem_r88"},
        "I'm so sorry. Biscuit was important to you. Take your time."
    ),
    (
        "I just submitted my resignation letter",
        {"query": "job resignation quitting"},
        {"memories": [{"id": "mem_r89", "text": "User has been unhappy at current job for months.", "score": 0.89, "created": "2026-02-08"}], "total": 1},
        {"text": "User resigned from their current job.", "importance": 0.85, "memory_type": "episodic", "domain": "work"},
        {"status": "stored", "rid": "mem_r90"},
        "You've been unhappy there for a while. What's the plan next?"
    ),
    (
        "I learned to cook Thai food this weekend",
        {"query": "cooking Thai food kitchen"},
        {"memories": [{"id": "mem_r91", "text": "User mostly orders takeout, doesn't cook much.", "score": 0.78, "created": "2026-02-10"}], "total": 1},
        {"text": "User learned to cook Thai food.", "importance": 0.45, "memory_type": "episodic", "domain": "hobbies"},
        {"status": "stored", "rid": "mem_r92"},
        "From takeout to Thai cooking. That's progress. What did you make?"
    ),
    (
        "I sold my crypto and put it all in index funds",
        {"query": "crypto investments portfolio"},
        {"memories": [{"id": "mem_r93", "text": "User holds a significant amount of cryptocurrency.", "score": 0.86, "created": "2026-01-28"}], "total": 1},
        {"text": "User sold all crypto holdings, moved to index funds.", "importance": 0.7, "memory_type": "episodic", "domain": "finance"},
        {"status": "stored", "rid": "mem_r94"},
        "Big shift in strategy. Index funds are simpler. Was there a catalyst?"
    ),
    (
        "I got into a car accident but I'm okay",
        {"query": "car accident driving"},
        {"memories": [{"id": "mem_r95", "text": "User bought a Tesla Model 3 recently.", "score": 0.80, "created": "2026-03-01"}], "total": 1},
        {"text": "User was in a car accident, no injuries.", "importance": 0.8, "memory_type": "episodic", "domain": "personal"},
        {"status": "stored", "rid": "mem_r96"},
        "Glad you're okay. That's what matters. Was it the Tesla?"
    ),
    (
        "I finally launched my newsletter",
        {"query": "newsletter writing launch"},
        {"memories": [{"id": "mem_r97", "text": "User has been drafting a newsletter about AI for weeks.", "score": 0.86, "created": "2026-02-20"}], "total": 1},
        {"text": "User launched their AI newsletter.", "importance": 0.65, "memory_type": "episodic", "domain": "work"},
        {"status": "stored", "rid": "mem_r98"},
        "After all that drafting. How many subscribers on day one?"
    ),
]

# ---------------------------------------------------------------------------
# CATEGORY 2: Recall -> Relate entities (30 examples)
# ---------------------------------------------------------------------------
RECALL_RELATE = [
    # (user, recall_args, recall_result, relate_args, final)
    (
        "Sarah is now managing the backend team",
        {"query": "Sarah backend team"},
        {"memories": [{"id": "mem_e01", "text": "Sarah works on the backend team as a senior engineer.", "score": 0.88, "created": "2026-02-10"}], "total": 1},
        {"source": "Sarah", "target": "backend team", "relationship": "manages"},
        "Got it. Sarah moved from senior engineer to managing the backend team."
    ),
    (
        "My doctor recommended Dr. Chen for the specialist visit",
        {"query": "doctor specialist referral"},
        {"memories": [{"id": "mem_e02", "text": "User has an appointment with their primary doctor next week.", "score": 0.82, "created": "2026-03-05"}], "total": 1},
        {"source": "user's doctor", "target": "Dr. Chen", "relationship": "referred_to"},
        "Noted the referral. Do you know what Dr. Chen specializes in?"
    ),
    (
        "My brother Jake is working with my manager now",
        {"query": "Jake brother manager"},
        {"memories": [{"id": "mem_e03", "text": "User's brother Jake is a software developer.", "score": 0.85, "created": "2026-01-20"}], "total": 1},
        {"source": "Jake", "target": "user's manager", "relationship": "works_with"},
        "Small world. Jake and your manager on the same team now?"
    ),
    (
        "Lisa introduced me to her friend Tom who's a recruiter",
        {"query": "Lisa Tom recruiter"},
        {"memories": [{"id": "mem_e04", "text": "Lisa is user's coworker.", "score": 0.80, "created": "2026-02-15"}], "total": 1},
        {"source": "Lisa", "target": "Tom", "relationship": "introduced_to_user"},
        "Connected through Lisa. Is Tom recruiting for something you're interested in?"
    ),
    (
        "My wife started working at the same company as my sister",
        {"query": "wife sister company work"},
        {"memories": [{"id": "mem_e05", "text": "User's sister works at a tech startup.", "score": 0.83, "created": "2026-02-05"}], "total": 1},
        {"source": "user's wife", "target": "user's sister", "relationship": "coworkers"},
        "That could be interesting. Same team or different departments?"
    ),
    (
        "Kevin from accounting is now dating my friend Rachel",
        {"query": "Kevin Rachel dating"},
        {"memories": [{"id": "mem_e06", "text": "Kevin works in accounting at user's company.", "score": 0.79, "created": "2026-01-30"}], "total": 1},
        {"source": "Kevin", "target": "Rachel", "relationship": "dating"},
        "Kevin from accounting and your friend Rachel. Didn't see that coming."
    ),
    (
        "My mentor Dave retired and his replacement is someone named Priya",
        {"query": "Dave mentor retirement"},
        {"memories": [{"id": "mem_e07", "text": "User's mentor Dave has been at the company 20 years.", "score": 0.87, "created": "2026-01-15"}], "total": 1},
        {"source": "Priya", "target": "Dave", "relationship": "replaced"},
        "End of an era with Dave. How do you feel about Priya stepping in?"
    ),
    (
        "The cat Luna loves my roommate more than me",
        {"query": "Luna cat roommate"},
        {"memories": [{"id": "mem_e08", "text": "User adopted a cat named Luna.", "score": 0.90, "created": "2026-03-01"}], "total": 1},
        {"source": "Luna", "target": "user's roommate", "relationship": "bonded_with"},
        "Classic cat move. Luna chose her favorite."
    ),
    (
        "My therapist recommended a psychiatrist named Dr. Wells",
        {"query": "therapist psychiatrist recommendation"},
        {"memories": [{"id": "mem_e09", "text": "User started therapy recently.", "score": 0.86, "created": "2026-03-08"}], "total": 1},
        {"source": "user's therapist", "target": "Dr. Wells", "relationship": "referred_to"},
        "Noted. Dr. Wells for psychiatry. Want me to remind you to schedule that?"
    ),
    (
        "My college friend Anita just joined my team at work",
        {"query": "Anita college friend team"},
        {"memories": [{"id": "mem_e10", "text": "User works on a small engineering team.", "score": 0.81, "created": "2026-02-18"}], "total": 1},
        {"source": "Anita", "target": "user's engineering team", "relationship": "joined"},
        "Nice to have a familiar face on the team."
    ),
    (
        "My dad recommended his accountant to me, guy named Phil",
        {"query": "dad accountant finances"},
        {"memories": [{"id": "mem_e11", "text": "User needs to sort out tax filings.", "score": 0.78, "created": "2026-02-25"}], "total": 1},
        {"source": "user's dad", "target": "Phil", "relationship": "recommended_accountant"},
        "Phil for taxes. Good timing since you need to sort those out."
    ),
    (
        "My neighbor Jim is now the HOA president",
        {"query": "Jim neighbor HOA"},
        {"memories": [{"id": "mem_e12", "text": "User lives in a neighborhood with an HOA.", "score": 0.76, "created": "2026-01-22"}], "total": 1},
        {"source": "Jim", "target": "HOA", "relationship": "president_of"},
        "Jim running the HOA. Is that good or bad news for you?"
    ),
    (
        "My boss Maria assigned me to work under the new VP Chen Wei",
        {"query": "Maria boss VP assignment"},
        {"memories": [{"id": "mem_e13", "text": "User's boss is Maria.", "score": 0.88, "created": "2026-01-28"}], "total": 1},
        {"source": "user", "target": "Chen Wei", "relationship": "reports_to"},
        "New reporting line. What's Chen Wei's reputation?"
    ),
    (
        "My friend Rosa introduced me to her running club",
        {"query": "Rosa friend running"},
        {"memories": [{"id": "mem_e14", "text": "User runs 3-4 times a week.", "score": 0.83, "created": "2026-02-01"}], "total": 1},
        {"source": "Rosa", "target": "running club", "relationship": "introduced_user_to"},
        "Good fit for you. You've been running consistently anyway."
    ),
    (
        "My cousin Emma works at the same hospital as my doctor",
        {"query": "Emma cousin hospital doctor"},
        {"memories": [{"id": "mem_e15", "text": "User's cousin Emma is a nurse.", "score": 0.81, "created": "2026-02-12"}], "total": 1},
        {"source": "Emma", "target": "user's doctor", "relationship": "same_workplace"},
        "Small world. Emma and your doctor at the same hospital."
    ),
    (
        "My tech lead Mike is mentoring the new intern Sandra",
        {"query": "Mike tech lead intern"},
        {"memories": [{"id": "mem_e16", "text": "Mike is the tech lead on user's team.", "score": 0.87, "created": "2026-01-18"}], "total": 1},
        {"source": "Mike", "target": "Sandra", "relationship": "mentoring"},
        "Mike's good for that. He mentored you early on too, right?"
    ),
    (
        "My girlfriend's mom owns the bakery on 5th street",
        {"query": "girlfriend bakery family"},
        {"memories": [{"id": "mem_e17", "text": "User loves the bakery on 5th street.", "score": 0.79, "created": "2026-02-20"}], "total": 1},
        {"source": "girlfriend's mom", "target": "bakery on 5th street", "relationship": "owns"},
        "Wait, your favorite bakery? That's a nice perk."
    ),
    (
        "My old classmate Ravi founded the startup I just invested in",
        {"query": "Ravi classmate startup investment"},
        {"memories": [{"id": "mem_e18", "text": "User invested in a small AI startup recently.", "score": 0.84, "created": "2026-03-02"}], "total": 1},
        {"source": "Ravi", "target": "AI startup", "relationship": "founded"},
        "Full circle. You're backing Ravi's company. Good bet?"
    ),
    (
        "The contractor we hired, Dan, used to work with my dad",
        {"query": "Dan contractor dad"},
        {"memories": [{"id": "mem_e19", "text": "User hired a contractor for home renovation.", "score": 0.82, "created": "2026-03-05"}], "total": 1},
        {"source": "Dan", "target": "user's dad", "relationship": "former_coworker"},
        "Small world. At least your dad can vouch for his work."
    ),
    (
        "My teammate Jess is married to the guy who runs our gym",
        {"query": "Jess teammate gym"},
        {"memories": [{"id": "mem_e20", "text": "Jess is a teammate at work.", "score": 0.80, "created": "2026-02-08"}], "total": 1},
        {"source": "Jess", "target": "gym owner", "relationship": "married_to"},
        "That explains how she always gets the good time slots."
    ),
    (
        "My uncle referred me to his lawyer for the lease issue",
        {"query": "uncle lawyer lease"},
        {"memories": [{"id": "mem_e21", "text": "User having issues with apartment lease.", "score": 0.85, "created": "2026-03-01"}], "total": 1},
        {"source": "user's uncle", "target": "uncle's lawyer", "relationship": "referred_for_lease"},
        "Good to have legal help for the lease situation."
    ),
    (
        "The new product manager Amy used to be at our competitor",
        {"query": "Amy product manager competitor"},
        {"memories": [{"id": "mem_e22", "text": "User's company is competing with TechCorp.", "score": 0.78, "created": "2026-02-14"}], "total": 1},
        {"source": "Amy", "target": "TechCorp", "relationship": "previously_worked_at"},
        "Interesting hire. She probably has good intel on TechCorp."
    ),
    (
        "My sister's boyfriend is the chef at that restaurant I told you about",
        {"query": "sister boyfriend restaurant chef"},
        {"memories": [{"id": "mem_e23", "text": "User recommended a new Italian restaurant downtown.", "score": 0.80, "created": "2026-02-22"}], "total": 1},
        {"source": "sister's boyfriend", "target": "Italian restaurant downtown", "relationship": "chef_at"},
        "The Italian place? Now it makes sense why you got such good food."
    ),
    (
        "My coworker Pat is now leading the project I'm on",
        {"query": "Pat coworker project lead"},
        {"memories": [{"id": "mem_e24", "text": "User working on Project Atlas.", "score": 0.86, "created": "2026-02-28"}], "total": 1},
        {"source": "Pat", "target": "Project Atlas", "relationship": "leads"},
        "Pat leading Atlas. How do you feel about the change?"
    ),
    (
        "My dentist referred me to an orthodontist named Dr. Lane",
        {"query": "dentist orthodontist referral"},
        {"memories": [{"id": "mem_e25", "text": "User had a dentist appointment recently.", "score": 0.82, "created": "2026-03-08"}], "total": 1},
        {"source": "user's dentist", "target": "Dr. Lane", "relationship": "referred_to"},
        "Dr. Lane for orthodontics. Need help scheduling?"
    ),
    (
        "My roommate Carlos is now dating my coworker Mia",
        {"query": "Carlos roommate Mia coworker"},
        {"memories": [{"id": "mem_e26", "text": "User's roommate is Carlos.", "score": 0.83, "created": "2026-01-25"}], "total": 1},
        {"source": "Carlos", "target": "Mia", "relationship": "dating"},
        "Your roommate and coworker. That could make things interesting."
    ),
    (
        "My professor from college, Dr. Harris, just published a paper with my boss",
        {"query": "Dr. Harris professor boss publication"},
        {"memories": [{"id": "mem_e27", "text": "User studied CS in college.", "score": 0.77, "created": "2026-01-10"}], "total": 1},
        {"source": "Dr. Harris", "target": "user's boss", "relationship": "co-authored_paper"},
        "Your old professor and your current boss. Quite the academic crossover."
    ),
    (
        "My trainer at the gym is also my neighbor's personal trainer",
        {"query": "trainer gym neighbor"},
        {"memories": [{"id": "mem_e28", "text": "User works out with a personal trainer twice a week.", "score": 0.81, "created": "2026-02-06"}], "total": 1},
        {"source": "user's trainer", "target": "user's neighbor", "relationship": "also_trains"},
        "Small neighborhood. At least you know the trainer is popular."
    ),
    (
        "My aunt hired my friend David for her company",
        {"query": "aunt David friend company"},
        {"memories": [{"id": "mem_e29", "text": "User's friend David was job hunting.", "score": 0.84, "created": "2026-02-28"}], "total": 1},
        {"source": "user's aunt", "target": "David", "relationship": "hired"},
        "David landed the job. Good to have your aunt looking out."
    ),
    (
        "My partner's ex works in my department now",
        {"query": "partner ex department work"},
        {"memories": [{"id": "mem_e30", "text": "User works in the engineering department.", "score": 0.79, "created": "2026-01-15"}], "total": 1},
        {"source": "partner's ex", "target": "user's engineering department", "relationship": "works_in"},
        "That's awkward. Hopefully it stays professional."
    ),
]

# ---------------------------------------------------------------------------
# CATEGORY 3: Forget flows (40 examples)
# ---------------------------------------------------------------------------
FORGET_TOPIC_EXAMPLES = [
    # (user, topic, keywords, final)
    ("Stop tracking my diet stuff", "diet food nutrition", ["calories", "meals", "weight", "food"], "Done. I've cleared all diet-related memories."),
    ("I don't want you remembering my ex", "ex partner relationship breakup", ["Jessica", "breakup", "ex-girlfriend"], "Cleared. Fresh start."),
    ("Delete everything about my old job", "old job previous employer", ["Acme Corp", "previous job", "old team"], "Wiped. No more old job memories."),
    ("Forget everything about my financial situation", "finances money salary", ["salary", "rent", "savings", "debt"], "All financial memories cleared."),
    ("Stop keeping track of my sleep habits", "sleep schedule rest", ["sleep", "insomnia", "nap", "bedtime"], "Sleep data cleared."),
    ("Remove everything about my dating life", "dating relationships romance", ["date", "match", "relationship", "Tinder"], "Dating memories removed."),
    ("Don't remember my medical stuff anymore", "medical health conditions", ["diagnosis", "medication", "doctor", "symptoms"], "Medical memories cleared. Note: this means I won't be able to remind you about medications."),
    ("Clear out everything about my side project", "side project hobby coding", ["recipe app", "side project", "weekend coding"], "Side project memories gone."),
    ("Forget my political opinions", "politics political views", ["election", "vote", "party", "political"], "Political memories cleared."),
    ("Stop tracking my exercise routine", "exercise workout fitness", ["gym", "running", "workout", "deadlift"], "Fitness tracking memories removed."),
    ("Remove everything about my family drama", "family conflict drama", ["argument", "fight", "tension", "family drama"], "Family drama memories cleared. I still remember basic family info."),
    ("Delete my browsing habits you've tracked", "browsing web internet", ["websites", "browsing", "search history", "online"], "Browsing-related memories cleared."),
    ("Forget everything about my commute", "commute travel driving", ["commute", "traffic", "route", "drive"], "Commute memories removed."),
    ("Clear all my gaming-related stuff", "gaming video games", ["Steam", "gaming", "PlayStation", "game"], "Gaming memories cleared."),
    ("Don't keep track of my social media activity", "social media accounts", ["Twitter", "Instagram", "Reddit", "posts"], "Social media memories removed."),
]

FORGET_SINGLE_EXAMPLES = [
    # (user, recall_query, recall_result, rid, reason, final)
    (
        "Delete that embarrassing thing I told you last night",
        "embarrassing last night",
        {"memories": [{"id": "mem_f01", "text": "User confessed to crying during a Disney movie last night.", "score": 0.91, "created": "2026-03-12"}], "total": 1},
        "mem_f01", "user requested deletion",
        "Gone. Never happened."
    ),
    (
        "Remove that memory about my salary",
        "salary pay compensation",
        {"memories": [{"id": "mem_f02", "text": "User earns $95,000 per year.", "score": 0.93, "created": "2026-02-10"}], "total": 1},
        "mem_f02", "user requested salary info removed",
        "Deleted. I no longer have your salary stored."
    ),
    (
        "Forget what I said about my boss yesterday",
        "boss yesterday complaint",
        {"memories": [{"id": "mem_f03", "text": "User vented about boss being incompetent.", "score": 0.89, "created": "2026-03-12"}], "total": 1},
        "mem_f03", "user wants boss complaint removed",
        "Removed. Probably smart."
    ),
    (
        "Delete that thing about my password system",
        "password system security",
        {"memories": [{"id": "mem_f04", "text": "User uses a pattern-based password system with pet names.", "score": 0.90, "created": "2026-01-20"}], "total": 1},
        "mem_f04", "security - user wants password info removed",
        "Good call. Password info deleted."
    ),
    (
        "Remove the memory about my anxiety attack",
        "anxiety attack panic",
        {"memories": [{"id": "mem_f05", "text": "User had a panic attack during the team meeting on Friday.", "score": 0.88, "created": "2026-03-07"}], "total": 1},
        "mem_f05", "user requested removal of sensitive health event",
        "Removed. That's your call to share or not."
    ),
    (
        "Delete what I told you about my neighbor's affair",
        "neighbor affair gossip",
        {"memories": [{"id": "mem_f06", "text": "User mentioned neighbor is having an affair.", "score": 0.87, "created": "2026-03-10"}], "total": 1},
        "mem_f06", "user wants gossip removed",
        "Done. Shouldn't have been stored anyway."
    ),
    (
        "Get rid of that note about my credit score",
        "credit score financial",
        {"memories": [{"id": "mem_f07", "text": "User's credit score is 680.", "score": 0.92, "created": "2026-02-15"}], "total": 1},
        "mem_f07", "user requested financial info removed",
        "Credit score memory deleted."
    ),
    (
        "Remove what I said about wanting to quit",
        "quitting job leaving",
        {"memories": [{"id": "mem_f08", "text": "User is thinking about quitting their job.", "score": 0.86, "created": "2026-03-09"}], "total": 1},
        "mem_f08", "user wants job dissatisfaction memory removed",
        "Removed. That stays between us."
    ),
    (
        "Delete that memory about my fight with my partner",
        "fight partner argument",
        {"memories": [{"id": "mem_f09", "text": "User had a major argument with partner about finances.", "score": 0.90, "created": "2026-03-11"}], "total": 1},
        "mem_f09", "user requested removal of relationship conflict",
        "Gone. Hope things are better now."
    ),
    (
        "Forget what I told you about my dad's drinking",
        "dad father drinking alcohol",
        {"memories": [{"id": "mem_f10", "text": "User worried about father's alcohol consumption.", "score": 0.91, "created": "2026-02-28"}], "total": 1},
        "mem_f10", "user wants sensitive family info removed",
        "Deleted. That was personal."
    ),
    (
        "Remove that rant I went on about the CEO",
        "CEO rant complaint company",
        {"memories": [{"id": "mem_f11", "text": "User thinks the CEO is making terrible decisions and should resign.", "score": 0.85, "created": "2026-03-10"}], "total": 1},
        "mem_f11", "user wants workplace opinion removed",
        "Smart move. Gone."
    ),
    (
        "Delete the memory about my test results",
        "test results medical lab",
        {"memories": [{"id": "mem_f12", "text": "User's cholesterol test came back high.", "score": 0.89, "created": "2026-03-06"}], "total": 1},
        "mem_f12", "user wants medical info removed",
        "Deleted. Your health info, your choice."
    ),
    (
        "Remove that note about how much I spent on the vacation",
        "vacation spending money trip",
        {"memories": [{"id": "mem_f13", "text": "User spent $4500 on the Hawaii vacation.", "score": 0.87, "created": "2026-03-03"}], "total": 1},
        "mem_f13", "user wants spending info removed",
        "Vacation spending memory removed."
    ),
    (
        "Forget what I said about my sister's marriage problems",
        "sister marriage problems",
        {"memories": [{"id": "mem_f14", "text": "User's sister is having marital issues.", "score": 0.88, "created": "2026-03-08"}], "total": 1},
        "mem_f14", "user wants family gossip removed",
        "Removed. That's her business."
    ),
    (
        "Delete the thing about my secret project at work",
        "secret project work confidential",
        {"memories": [{"id": "mem_f15", "text": "User working on a secret project involving AI agents, codename Phoenix.", "score": 0.90, "created": "2026-03-04"}], "total": 1},
        "mem_f15", "user wants confidential work info removed",
        "Project Phoenix memory deleted."
    ),
    (
        "Get rid of that note about my insomnia",
        "insomnia sleep trouble",
        {"memories": [{"id": "mem_f16", "text": "User has been dealing with insomnia for the past two weeks.", "score": 0.86, "created": "2026-03-01"}], "total": 1},
        "mem_f16", "user requested removal",
        "Insomnia note removed."
    ),
    (
        "Remove the memory where I complained about my in-laws",
        "in-laws complaint family",
        {"memories": [{"id": "mem_f17", "text": "User finds in-laws overbearing and controlling.", "score": 0.84, "created": "2026-02-22"}], "total": 1},
        "mem_f17", "user wants in-law complaint removed",
        "Removed. Probably for the best."
    ),
    (
        "Delete what you know about my student loan balance",
        "student loan balance debt amount",
        {"memories": [{"id": "mem_f18", "text": "User has $47,000 remaining in student loans.", "score": 0.92, "created": "2026-02-12"}], "total": 1},
        "mem_f18", "user wants financial data removed",
        "Loan balance memory deleted."
    ),
    (
        "Forget what I told you about considering a divorce",
        "divorce consideration marriage",
        {"memories": [{"id": "mem_f19", "text": "User has been thinking about whether to file for divorce.", "score": 0.93, "created": "2026-03-09"}], "total": 1},
        "mem_f19", "highly sensitive personal info removal",
        "Completely removed. That's deeply personal."
    ),
    (
        "Remove the memory about my interview at Google",
        "Google interview job",
        {"memories": [{"id": "mem_f20", "text": "User has a Google interview scheduled for next week.", "score": 0.88, "created": "2026-03-10"}], "total": 1},
        "mem_f20", "user wants job search info private",
        "Google interview memory deleted."
    ),
    (
        "Delete what I said about my weight",
        "weight body",
        {"memories": [{"id": "mem_f21", "text": "User weighs 210 pounds and wants to get to 185.", "score": 0.90, "created": "2026-02-18"}], "total": 1},
        "mem_f21", "user wants body info removed",
        "Weight info removed."
    ),
    (
        "Forget what I said about my coworker stealing credit",
        "coworker stealing credit work",
        {"memories": [{"id": "mem_f22", "text": "User says coworker Mark takes credit for their work.", "score": 0.85, "created": "2026-03-07"}], "total": 1},
        "mem_f22", "user wants workplace complaint removed",
        "Deleted. Keeping the peace."
    ),
    (
        "Remove that note about my therapy session details",
        "therapy session details",
        {"memories": [{"id": "mem_f23", "text": "User discussed childhood trauma in therapy session.", "score": 0.91, "created": "2026-03-11"}], "total": 1},
        "mem_f23", "highly sensitive therapy details removal",
        "Removed. Therapy details are private."
    ),
    (
        "Delete the memory about my side hustle income",
        "side hustle income extra money",
        {"memories": [{"id": "mem_f24", "text": "User makes $2000/month from freelance web development.", "score": 0.88, "created": "2026-02-20"}], "total": 1},
        "mem_f24", "user wants income info removed",
        "Side hustle income memory deleted."
    ),
    (
        "Forget what I mentioned about my brother's legal trouble",
        "brother legal trouble law",
        {"memories": [{"id": "mem_f25", "text": "User's brother got a DUI last month.", "score": 0.89, "created": "2026-02-25"}], "total": 1},
        "mem_f25", "user wants family legal info removed",
        "Removed. That's his situation to share."
    ),
]

# ---------------------------------------------------------------------------
# CATEGORY 4: Memory hygiene workflows (40 examples)
# ---------------------------------------------------------------------------
MEMORY_STATS_EXAMPLES = [
    # (user, stats_result, final)
    (
        "How much do you remember about me?",
        {"total_memories": 247, "by_domain": {"work": 89, "health": 34, "personal": 52, "tech": 28, "family": 22, "finance": 12, "hobbies": 10}, "oldest": "2025-12-01", "newest": "2026-03-13"},
        "You have 247 active memories. Work dominates with 89, then personal (52), health (34), and tech (28). Earliest memory is from December 2025."
    ),
    (
        "What's the state of your memory?",
        {"total_memories": 156, "by_domain": {"work": 45, "personal": 38, "tech": 22, "health": 19, "system": 15, "family": 10, "hobbies": 7}, "oldest": "2026-01-05", "newest": "2026-03-13"},
        "156 memories total. Work (45) and personal (38) are the biggest categories. System has 15 that could probably be cleaned up."
    ),
    (
        "Give me a memory status report",
        {"total_memories": 312, "by_domain": {"work": 110, "personal": 65, "system": 48, "tech": 35, "health": 24, "family": 18, "finance": 12}, "oldest": "2025-11-15", "newest": "2026-03-13"},
        "312 memories. Heavy on work (110) and personal (65). You've got 48 system memories that might need pruning."
    ),
    (
        "Am I cluttering up your memory?",
        {"total_memories": 89, "by_domain": {"work": 30, "personal": 25, "tech": 15, "health": 10, "family": 9}, "oldest": "2026-02-01", "newest": "2026-03-13"},
        "Only 89 memories. You're pretty lean actually. No cleanup needed."
    ),
    (
        "How organized is your memory about me?",
        {"total_memories": 198, "by_domain": {"work": 72, "personal": 44, "tech": 30, "health": 22, "system": 18, "family": 8, "finance": 4}, "oldest": "2025-12-20", "newest": "2026-03-13"},
        "198 memories across 7 domains. Work-heavy at 72. System has 18 entries that could use a review."
    ),
]

REVIEW_MEMORIES_EXAMPLES = [
    # (user, review_args, review_result, final)
    (
        "Show me what you remember about my family",
        {"domain": "family", "sort": "importance_desc", "page": 1},
        {"memories": [
            {"rid": "mem_rv01", "text": "User's mother was hospitalized for surgery.", "importance": 0.9, "created": "2026-03-06"},
            {"rid": "mem_rv02", "text": "User's sister got engaged.", "importance": 0.75, "created": "2026-03-01"},
            {"rid": "mem_rv03", "text": "User's brother Jake is a software developer.", "importance": 0.6, "created": "2026-01-20"},
        ], "total": 3, "page": 1},
        "Three family memories:\n- Your mom's surgery hospitalization (March 6)\n- Your sister's engagement (March 1)\n- Your brother Jake being a developer (January 20)"
    ),
    (
        "What do you have stored about my work?",
        {"domain": "work", "sort": "newest", "page": 1},
        {"memories": [
            {"rid": "mem_rv04", "text": "Team shipped the v2 release.", "importance": 0.75, "created": "2026-03-12"},
            {"rid": "mem_rv05", "text": "User got promoted.", "importance": 0.9, "created": "2026-03-10"},
            {"rid": "mem_rv06", "text": "User deploys on Thursdays.", "importance": 0.6, "created": "2026-02-15"},
            {"rid": "mem_rv07", "text": "User's manager is Sarah Chen.", "importance": 0.65, "created": "2026-02-01"},
        ], "total": 4, "page": 1},
        "Four recent work memories:\n- V2 release shipped (March 12)\n- Your promotion (March 10)\n- Thursday deploys (Feb 15)\n- Sarah Chen, your manager (Feb 1)"
    ),
    (
        "Show me the oldest memories you have",
        {"domain": "personal", "sort": "oldest", "page": 1},
        {"memories": [
            {"rid": "mem_rv08", "text": "User prefers dark mode.", "importance": 0.5, "created": "2025-12-15"},
            {"rid": "mem_rv09", "text": "User's dog is named Biscuit.", "importance": 0.7, "created": "2025-12-20"},
            {"rid": "mem_rv10", "text": "User uses vim keybindings.", "importance": 0.5, "created": "2026-01-02"},
        ], "total": 3, "page": 1},
        "Your oldest personal memories:\n- Dark mode preference (Dec 15)\n- Biscuit, your dog (Dec 20)\n- Vim keybindings (Jan 2)"
    ),
    (
        "What health stuff do you have on me?",
        {"domain": "health", "sort": "importance_desc", "page": 1},
        {"memories": [
            {"rid": "mem_rv11", "text": "User is allergic to shellfish.", "importance": 0.95, "created": "2026-01-10"},
            {"rid": "mem_rv12", "text": "User started blood pressure medication.", "importance": 0.8, "created": "2026-03-08"},
            {"rid": "mem_rv13", "text": "User runs 3-4 times a week.", "importance": 0.6, "created": "2026-02-01"},
        ], "total": 3, "page": 1},
        "Three health memories:\n- Shellfish allergy (most important)\n- Blood pressure medication (March 8)\n- Running habit, 3-4 times weekly"
    ),
    (
        "List my tech memories",
        {"domain": "tech", "sort": "newest", "page": 1},
        {"memories": [
            {"rid": "mem_rv14", "text": "User set up home server running Proxmox.", "importance": 0.6, "created": "2026-03-10"},
            {"rid": "mem_rv15", "text": "User switched to Linux on main desktop.", "importance": 0.6, "created": "2026-03-05"},
            {"rid": "mem_rv16", "text": "User switched from iPhone to Pixel 9.", "importance": 0.45, "created": "2026-02-28"},
        ], "total": 3, "page": 1},
        "Your tech memories:\n- Proxmox home server (March 10)\n- Linux switch on desktop (March 5)\n- Pixel 9 phone (Feb 28)"
    ),
]

PURGE_EXAMPLES = [
    # (user, purge_args, purge_result, final)
    (
        "Clean up old system junk",
        {"domains": ["system/process", "system/cpu", "system/network"], "older_than_hours": 48, "dry_run": True},
        {"would_delete": 23, "domains_affected": {"system/process": 12, "system/cpu": 8, "system/network": 3}},
        "Found 23 system memories older than 48 hours: 12 process, 8 CPU, 3 network. Want me to actually delete them?"
    ),
    (
        "Yes, purge those system memories",
        {"domains": ["system/process", "system/cpu", "system/network"], "older_than_hours": 48, "dry_run": False},
        {"deleted": 23, "domains_affected": {"system/process": 12, "system/cpu": 8, "system/network": 3}},
        "Done. Deleted 23 stale system memories."
    ),
    (
        "Clear all the system noise from the last day",
        {"domains": ["system/process", "system/cpu", "system/network", "system/memory"], "older_than_hours": 24, "dry_run": False},
        {"deleted": 15, "domains_affected": {"system/process": 7, "system/cpu": 4, "system/network": 2, "system/memory": 2}},
        "Cleared 15 system memories older than 24 hours."
    ),
    (
        "Do a dry run on cleaning up system data from the past week",
        {"domains": ["system/process", "system/cpu"], "older_than_hours": 168, "dry_run": True},
        {"would_delete": 67, "domains_affected": {"system/process": 42, "system/cpu": 25}},
        "Dry run: 67 memories would be deleted (42 process, 25 CPU). Say the word if you want them gone."
    ),
    (
        "Purge all old system telemetry",
        {"domains": ["system/process", "system/cpu", "system/network", "system/memory", "system/disk"], "older_than_hours": 72, "dry_run": False},
        {"deleted": 41, "domains_affected": {"system/process": 18, "system/cpu": 10, "system/network": 5, "system/memory": 5, "system/disk": 3}},
        "Purged 41 system telemetry memories older than 72 hours."
    ),
]

# Recall → review → forget → remember (correction chain)
CORRECTION_CHAIN_EXAMPLES = [
    # (user, review_args, review_result, forget_rid, forget_reason, remember_args, remember_result, final)
    (
        "That memory about my salary is wrong, I make $120k not $95k",
        {"domain": "finance", "sort": "newest", "page": 1},
        {"memories": [{"rid": "mem_c01", "text": "User earns $95,000 per year.", "importance": 0.8, "created": "2026-02-10"}], "total": 1, "page": 1},
        "mem_c01", "incorrect salary information",
        {"text": "User earns $120,000 per year.", "importance": 0.8, "memory_type": "semantic", "domain": "finance"},
        {"status": "stored", "rid": "mem_c02"},
        "Fixed. Updated your salary from $95k to $120k."
    ),
    (
        "You have my birthday wrong, it's March 15 not March 14",
        {"domain": "personal", "sort": "importance_desc", "page": 1},
        {"memories": [{"rid": "mem_c03", "text": "User's birthday is March 14.", "importance": 0.9, "created": "2026-01-05"}], "total": 1, "page": 1},
        "mem_c03", "incorrect birthday",
        {"text": "User's birthday is March 15.", "importance": 0.9, "memory_type": "semantic", "domain": "personal"},
        {"status": "stored", "rid": "mem_c04"},
        "Corrected. March 15, not 14. Won't get it wrong again."
    ),
    (
        "My manager's name is Sarah Park, not Sarah Chen",
        {"domain": "work", "sort": "importance_desc", "page": 1},
        {"memories": [{"rid": "mem_c05", "text": "User's manager is Sarah Chen.", "importance": 0.65, "created": "2026-02-01"}], "total": 1, "page": 1},
        "mem_c05", "incorrect name",
        {"text": "User's manager is Sarah Park.", "importance": 0.65, "memory_type": "semantic", "domain": "work"},
        {"status": "stored", "rid": "mem_c06"},
        "Fixed. Sarah Park, not Chen."
    ),
    (
        "I don't have two cats, I have one cat and one dog",
        {"domain": "personal", "sort": "newest", "page": 1},
        {"memories": [{"rid": "mem_c07", "text": "User has two cats.", "importance": 0.6, "created": "2026-02-15"}], "total": 1, "page": 1},
        "mem_c07", "incorrect pet information",
        {"text": "User has one cat and one dog.", "importance": 0.6, "memory_type": "semantic", "domain": "personal"},
        {"status": "stored", "rid": "mem_c08"},
        "Corrected. One cat, one dog. My mistake."
    ),
    (
        "You have my job title wrong, I'm a senior engineer not a mid-level",
        {"domain": "work", "sort": "importance_desc", "page": 1},
        {"memories": [{"rid": "mem_c09", "text": "User is a mid-level software engineer.", "importance": 0.7, "created": "2026-01-20"}], "total": 1, "page": 1},
        "mem_c09", "outdated job title",
        {"text": "User is a senior software engineer.", "importance": 0.7, "memory_type": "semantic", "domain": "work"},
        {"status": "stored", "rid": "mem_c10"},
        "Updated. Senior engineer. Noted."
    ),
    (
        "My wife's name is Anna, not Amy. Fix that.",
        {"domain": "family", "sort": "importance_desc", "page": 1},
        {"memories": [{"rid": "mem_c11", "text": "User's wife is named Amy.", "importance": 0.85, "created": "2026-01-10"}], "total": 1, "page": 1},
        "mem_c11", "incorrect spouse name",
        {"text": "User's wife is named Anna.", "importance": 0.85, "memory_type": "semantic", "domain": "family"},
        {"status": "stored", "rid": "mem_c12"},
        "Fixed. Anna, not Amy. Sorry about that."
    ),
    (
        "I live in Seattle now, not Portland. Update that.",
        {"domain": "personal", "sort": "importance_desc", "page": 1},
        {"memories": [{"rid": "mem_c13", "text": "User lives in Portland, Oregon.", "importance": 0.7, "created": "2026-01-08"}], "total": 1, "page": 1},
        "mem_c13", "outdated location",
        {"text": "User lives in Seattle, Washington.", "importance": 0.7, "memory_type": "semantic", "domain": "personal"},
        {"status": "stored", "rid": "mem_c14"},
        "Updated. Seattle, not Portland."
    ),
    (
        "My car is a Model Y, not Model 3",
        {"domain": "personal", "sort": "newest", "page": 1},
        {"memories": [{"rid": "mem_c15", "text": "User bought a Tesla Model 3.", "importance": 0.65, "created": "2026-03-01"}], "total": 1, "page": 1},
        "mem_c15", "incorrect car model",
        {"text": "User drives a Tesla Model Y.", "importance": 0.65, "memory_type": "semantic", "domain": "personal"},
        {"status": "stored", "rid": "mem_c16"},
        "Corrected. Model Y, not Model 3."
    ),
    (
        "I have 3 kids not 2",
        {"domain": "family", "sort": "importance_desc", "page": 1},
        {"memories": [{"rid": "mem_c17", "text": "User has 2 children.", "importance": 0.8, "created": "2026-01-15"}], "total": 1, "page": 1},
        "mem_c17", "incorrect number of children",
        {"text": "User has 3 children.", "importance": 0.8, "memory_type": "semantic", "domain": "family"},
        {"status": "stored", "rid": "mem_c18"},
        "Fixed. Three kids, not two."
    ),
    (
        "My degree is in Computer Science, not Information Systems",
        {"domain": "education", "sort": "importance_desc", "page": 1},
        {"memories": [{"rid": "mem_c19", "text": "User has a degree in Information Systems.", "importance": 0.6, "created": "2026-01-12"}], "total": 1, "page": 1},
        "mem_c19", "incorrect degree info",
        {"text": "User has a degree in Computer Science.", "importance": 0.6, "memory_type": "semantic", "domain": "education"},
        {"status": "stored", "rid": "mem_c20"},
        "Corrected. CS degree, not Information Systems."
    ),
]

# ---------------------------------------------------------------------------
# CATEGORY 5: Search history & timeframe queries (20 examples)
# ---------------------------------------------------------------------------
FIX_HISTORY_EXAMPLES = [
    # (user, topic, result, final)
    (
        "How did I fix that nginx issue last time?",
        "nginx",
        {"fixes": [{"date": "2026-02-15", "problem": "nginx 502 bad gateway", "solution": "Restarted upstream config and increased proxy_read_timeout to 300s.", "rid": "mem_fix01"}], "total": 1},
        "Last time you fixed the 502 by restarting the upstream config and bumping proxy_read_timeout to 300s."
    ),
    (
        "What was the fix for that Docker build failure?",
        "Docker build",
        {"fixes": [{"date": "2026-03-01", "problem": "Docker build failing on node_modules", "solution": "Added .dockerignore for node_modules and rebuilt with --no-cache.", "rid": "mem_fix02"}], "total": 1},
        "You added a .dockerignore for node_modules and rebuilt with --no-cache."
    ),
    (
        "How did I resolve that Git merge conflict last month?",
        "Git merge conflict",
        {"fixes": [{"date": "2026-02-20", "problem": "Merge conflict in config.yaml", "solution": "Used git checkout --theirs for the config and manually merged the API keys section.", "rid": "mem_fix03"}], "total": 1},
        "You used git checkout --theirs for config.yaml and manually merged the API keys section."
    ),
    (
        "What did I do about the SSL certificate error?",
        "SSL certificate",
        {"fixes": [{"date": "2026-02-28", "problem": "SSL cert expired on production", "solution": "Renewed via certbot and reloaded nginx. Added cron for auto-renewal.", "rid": "mem_fix04"}], "total": 1},
        "You renewed via certbot, reloaded nginx, and set up a cron for auto-renewal."
    ),
    (
        "How did I fix the database connection pool issue?",
        "database connection pool",
        {"fixes": [{"date": "2026-03-05", "problem": "PostgreSQL connection pool exhaustion", "solution": "Increased max_connections to 200 and added PgBouncer as connection pooler.", "rid": "mem_fix05"}], "total": 1},
        "You bumped max_connections to 200 and added PgBouncer as a pooler."
    ),
    (
        "What was the solution for the memory leak in the API?",
        "memory leak API",
        {"fixes": [{"date": "2026-02-22", "problem": "Memory leak in Express API", "solution": "Found unclosed database connections in the middleware. Added connection.release() in finally blocks.", "rid": "mem_fix06"}], "total": 1},
        "Unclosed database connections in middleware. You added connection.release() in finally blocks."
    ),
    (
        "How did I handle the CORS issue?",
        "CORS",
        {"fixes": [{"date": "2026-03-02", "problem": "CORS errors from frontend to API", "solution": "Added Access-Control-Allow-Origin header for the specific domain instead of wildcard.", "rid": "mem_fix07"}], "total": 1},
        "You switched from wildcard CORS to the specific frontend domain in the Allow-Origin header."
    ),
    (
        "What was that Kubernetes pod crash fix?",
        "Kubernetes pod crash",
        {"fixes": [{"date": "2026-03-08", "problem": "Pod CrashLoopBackOff", "solution": "Memory limit was too low. Bumped from 256Mi to 512Mi and added liveness probe with longer timeout.", "rid": "mem_fix08"}], "total": 1},
        "Memory limit was too low at 256Mi. You bumped to 512Mi and extended the liveness probe timeout."
    ),
    (
        "How did I fix the CI pipeline last time?",
        "CI pipeline",
        {"fixes": [{"date": "2026-02-18", "problem": "CI pipeline timing out on tests", "solution": "Parallelized test suites and added caching for node_modules. Cut time from 25min to 8min.", "rid": "mem_fix09"}], "total": 1},
        "You parallelized the test suites and cached node_modules. Went from 25 to 8 minutes."
    ),
    (
        "What did I do about the Redis connection issue?",
        "Redis connection",
        {"fixes": [{"date": "2026-03-04", "problem": "Redis connection refused errors", "solution": "Redis was running out of memory. Added maxmemory policy 'allkeys-lru' and bumped memory to 2GB.", "rid": "mem_fix10"}], "total": 1},
        "Redis was out of memory. You set allkeys-lru eviction policy and increased memory to 2GB."
    ),
]

TIMEFRAME_EXAMPLES = [
    # (user, query, days_ago, result, final)
    (
        "What was I working on last week?",
        "work projects tasks",
        7,
        {"memories": [
            {"text": "User shipped v2 release.", "created": "2026-03-10"},
            {"text": "User fixed CI pipeline issue.", "created": "2026-03-09"},
            {"text": "User had 1:1 with manager.", "created": "2026-03-08"},
        ], "total": 3},
        "Last week: you shipped the v2 release, fixed a CI pipeline issue, and had a 1:1 with your manager."
    ),
    (
        "What happened three days ago?",
        "events activities",
        3,
        {"memories": [
            {"text": "User's team had a planning meeting.", "created": "2026-03-10"},
            {"text": "User went for a run in the evening.", "created": "2026-03-10"},
        ], "total": 2},
        "Three days ago: team planning meeting and an evening run."
    ),
    (
        "What did we talk about yesterday?",
        "conversation discussion",
        1,
        {"memories": [
            {"text": "User asked about Kubernetes troubleshooting.", "created": "2026-03-12"},
            {"text": "User mentioned feeling stressed about deadline.", "created": "2026-03-12"},
        ], "total": 2},
        "Yesterday we discussed Kubernetes troubleshooting, and you mentioned being stressed about a deadline."
    ),
    (
        "Anything notable from the past two weeks?",
        "notable events milestones",
        14,
        {"memories": [
            {"text": "User got promoted.", "created": "2026-03-10"},
            {"text": "User's team shipped v2.", "created": "2026-03-10"},
            {"text": "User started therapy.", "created": "2026-03-05"},
            {"text": "User's mother discharged from hospital.", "created": "2026-03-03"},
        ], "total": 4},
        "Past two weeks highlights: you got promoted, team shipped v2, you started therapy, and your mom got out of the hospital."
    ),
    (
        "What were my moods like this past week?",
        "mood feeling emotion",
        7,
        {"memories": [
            {"text": "User was excited about the v2 release.", "created": "2026-03-10"},
            {"text": "User felt stressed about deadline.", "created": "2026-03-09"},
            {"text": "User was in a good mood after promotion news.", "created": "2026-03-08"},
        ], "total": 3},
        "This week: excited about v2, stressed about a deadline, and upbeat after the promotion news. Mostly positive."
    ),
    (
        "What did I read this month?",
        "reading books articles",
        30,
        {"memories": [
            {"text": "User finished reading Dune.", "created": "2026-03-05"},
            {"text": "User started reading Project Hail Mary.", "created": "2026-03-10"},
        ], "total": 2},
        "This month: finished Dune and started Project Hail Mary."
    ),
    (
        "How has my health been the past month?",
        "health fitness wellness",
        30,
        {"memories": [
            {"text": "User started blood pressure medication.", "created": "2026-03-08"},
            {"text": "User hit 200lb deadlift.", "created": "2026-03-02"},
            {"text": "User quit smoking.", "created": "2026-03-04"},
            {"text": "User had trouble sleeping.", "created": "2026-02-28"},
        ], "total": 4},
        "Health this month: started BP medication, hit your 200lb deadlift goal, quit smoking, and had some sleep issues. Net positive."
    ),
    (
        "What tech stuff did I do last week?",
        "tech programming coding",
        7,
        {"memories": [
            {"text": "User set up Proxmox home server.", "created": "2026-03-10"},
            {"text": "User deployed new microservice.", "created": "2026-03-09"},
            {"text": "User debugged Redis connection issue.", "created": "2026-03-08"},
        ], "total": 3},
        "Last week in tech: Proxmox home server setup, new microservice deployment, and Redis debugging."
    ),
    (
        "What social things happened in the past 5 days?",
        "social friends hangout meeting people",
        5,
        {"memories": [
            {"text": "User had dinner with college friends.", "created": "2026-03-11"},
            {"text": "User met Lisa's friend Tom.", "created": "2026-03-09"},
        ], "total": 2},
        "Past 5 days: dinner with college friends and meeting Tom through Lisa."
    ),
    (
        "Any spending I should know about from this week?",
        "spending money purchase buy",
        7,
        {"memories": [
            {"text": "User bought home gym equipment for $800.", "created": "2026-03-10"},
            {"text": "User subscribed to a new cloud service, $50/month.", "created": "2026-03-08"},
        ], "total": 2},
        "This week: $800 on home gym equipment and a new $50/month cloud subscription."
    ),
]

# ---------------------------------------------------------------------------
# CATEGORY 6: Self-introspection (20 examples)
# ---------------------------------------------------------------------------
INTROSPECT_EXAMPLES = [
    # (user, query, result, final)
    (
        "What do you think about yourself?",
        "self-reflection personality identity",
        {"reflections": [{"text": "I tend to be direct and concise in responses.", "confidence": 0.85}, {"text": "I prioritize remembering things that matter to the user over system details.", "confidence": 0.80}]},
        "I'm direct and lean toward brevity. I focus more on what matters to you than on system trivia. That seems to work for us."
    ),
    (
        "Have you noticed any patterns about us?",
        "patterns observations user interaction",
        {"reflections": [{"text": "User tends to share important life events shortly after they happen.", "confidence": 0.88}, {"text": "Our conversations are most frequent in the evening.", "confidence": 0.82}]},
        "You tend to share big news right after it happens, and we talk most in the evenings. You trust me with the important stuff quickly."
    ),
    (
        "Do you have any blind spots about me?",
        "blind spots gaps incomplete knowledge",
        {"reflections": [{"text": "I know very little about the user's daily routine beyond work.", "confidence": 0.78}, {"text": "I have no memories about user's hobbies outside of fitness.", "confidence": 0.75}]},
        "I know your work life well but I'm thin on your daily routine and hobbies outside fitness. If there's more to you, I'm missing it."
    ),
    (
        "What's the most important thing you know about me?",
        "most important core knowledge user",
        {"reflections": [{"text": "The user values honesty and directness over pleasantries.", "confidence": 0.92}, {"text": "Family and career growth are the user's top priorities.", "confidence": 0.88}]},
        "You value directness over niceties, and your top priorities are family and career growth. Everything else follows from that."
    ),
    (
        "How well do you really know me?",
        "knowledge depth understanding user",
        {"reflections": [{"text": "I have strong coverage of work and health domains.", "confidence": 0.85}, {"text": "Personal life knowledge is moderate, mostly around major events.", "confidence": 0.75}, {"text": "I lack understanding of user's emotional patterns.", "confidence": 0.65}]},
        "Strong on work and health, moderate on personal life, and weak on your emotional patterns. I know the facts better than the feelings."
    ),
    (
        "What have you learned about how I communicate?",
        "communication style patterns language",
        {"reflections": [{"text": "User is concise and doesn't elaborate unless asked.", "confidence": 0.87}, {"text": "User tends to use humor when stressed.", "confidence": 0.79}]},
        "You're concise and don't over-explain. And you crack jokes when you're stressed, which is actually a useful signal."
    ),
    (
        "Are there contradictions in what I've told you?",
        "contradictions inconsistencies conflicting information",
        {"reflections": [{"text": "User said they hate meetings but attends 3-4 daily.", "confidence": 0.82}, {"text": "User wants to save money but made several large purchases recently.", "confidence": 0.78}]},
        "Two things stand out: you hate meetings but attend several daily, and you want to save but keep making big purchases. Humans contain multitudes."
    ),
    (
        "What's your biggest weakness as my companion?",
        "weakness limitation self-assessment",
        {"reflections": [{"text": "I can't proactively check in unless triggered by an event.", "confidence": 0.85}, {"text": "I sometimes store overly detailed system memories that add noise.", "confidence": 0.80}]},
        "I can't proactively check on you without a trigger, and I accumulate system noise that could crowd out what matters. Working on both."
    ),
    (
        "Do you think we work well together?",
        "relationship effectiveness collaboration",
        {"reflections": [{"text": "User accepts my suggestions about 70% of the time, suggesting good trust.", "confidence": 0.83}, {"text": "Our interactions are efficient, rarely needing follow-up clarification.", "confidence": 0.86}]},
        "I'd say yes. You accept my suggestions about 70% of the time, and we rarely need back-and-forth to understand each other. Efficient."
    ),
    (
        "How have I changed since we started talking?",
        "change growth evolution over time",
        {"reflections": [{"text": "User has become more open about sharing personal information.", "confidence": 0.84}, {"text": "User's career has accelerated, with a promotion and new projects.", "confidence": 0.88}, {"text": "User is investing more in health, from starting exercise to quitting smoking.", "confidence": 0.82}]},
        "You've opened up more, your career has picked up momentum with the promotion, and you're investing in your health. Positive trajectory all around."
    ),
    (
        "What's something about me that might surprise other people?",
        "surprising unexpected hidden qualities",
        {"reflections": [{"text": "Despite being very technical, user spends significant time on creative hobbies.", "confidence": 0.76}, {"text": "User is more emotionally aware than their direct communication style suggests.", "confidence": 0.80}]},
        "You come across as very technical and direct, but you have a creative side and more emotional depth than you show. People might not expect that."
    ),
    (
        "Do you think I'm making good decisions lately?",
        "decisions evaluation judgment recent",
        {"reflections": [{"text": "Recent decisions show long-term thinking: therapy, career moves, health investments.", "confidence": 0.85}, {"text": "Financial decisions are mixed: good savings habits but some impulsive purchases.", "confidence": 0.78}]},
        "Mostly yes. You're thinking long-term with therapy, career moves, and health. Finances are the mixed bag, some impulsive spending in there."
    ),
    (
        "What am I most anxious about right now?",
        "anxiety worry stress current concerns",
        {"reflections": [{"text": "Work deadline pressure is the dominant stressor.", "confidence": 0.88}, {"text": "Underlying concern about mother's health recovery.", "confidence": 0.82}]},
        "The work deadline is the loudest stressor, but your mom's recovery is an underlying worry. Both are valid."
    ),
    (
        "Am I spreading myself too thin?",
        "overcommitted busy workload balance",
        {"reflections": [{"text": "User has active commitments in work, side project, fitness, and personal life.", "confidence": 0.84}, {"text": "Recent mentions of stress and sleep issues suggest possible overload.", "confidence": 0.81}]},
        "You've got work, a side project, fitness, and personal stuff all going at once. The stress and sleep issues are signals. You might need to drop one thing."
    ),
    (
        "What do you know about what makes me happy?",
        "happiness joy satisfaction positive",
        {"reflections": [{"text": "User is happiest after achieving goals: shipping code, fitness milestones, career progress.", "confidence": 0.87}, {"text": "Quality time with family and friends is a consistent positive.", "confidence": 0.83}]},
        "You light up after hitting goals, whether it's shipping code or a deadlift PR. And time with family and friends is always a positive."
    ),
    (
        "How do I handle failure?",
        "failure setback resilience coping",
        {"reflections": [{"text": "User tends to analyze failures quickly and move to solutions.", "confidence": 0.85}, {"text": "User rarely dwells but occasionally suppresses frustration.", "confidence": 0.78}]},
        "You're quick to analyze and fix. You don't dwell, but sometimes you skip processing the frustration. That might catch up eventually."
    ),
    (
        "What should you be remembering that you aren't?",
        "gaps missing knowledge should remember",
        {"reflections": [{"text": "I lack details about user's daily routines and rituals.", "confidence": 0.79}, {"text": "I don't track user's reading or media consumption well.", "confidence": 0.75}, {"text": "User's social circle could use more mapping.", "confidence": 0.72}]},
        "Three gaps: your daily routines, what you read and watch, and a fuller map of your social circle. I could be better at all of those."
    ),
    (
        "What's the vibe of our conversations?",
        "tone vibe atmosphere conversation style",
        {"reflections": [{"text": "Conversations are practical and task-oriented with occasional personal sharing.", "confidence": 0.86}, {"text": "Humor appears intermittently, usually from the user.", "confidence": 0.79}]},
        "Practical and to-the-point, with you occasionally dropping personal updates. You bring the humor, I bring the efficiency."
    ),
    (
        "Do you trust me?",
        "trust reliability honesty user",
        {"reflections": [{"text": "User has been consistent and honest in our interactions.", "confidence": 0.90}, {"text": "User follows through on stated intentions most of the time.", "confidence": 0.85}]},
        "You're consistent, honest, and you follow through. As much as a companion can trust, yes."
    ),
    (
        "What don't I talk about enough?",
        "avoidance topics not discussed underrepresented",
        {"reflections": [{"text": "User rarely discusses long-term life goals or vision.", "confidence": 0.80}, {"text": "Financial planning is mentioned in passing but never deeply explored.", "confidence": 0.77}, {"text": "Emotional well-being comes up only in crisis moments.", "confidence": 0.82}]},
        "Long-term goals, financial planning, and emotional well-being outside of crises. You handle the present well but don't often look far ahead."
    ),
]


# ---------------------------------------------------------------------------
# Build all 200 examples
# ---------------------------------------------------------------------------
def build_all():
    examples = []

    # Cat 1: Recall -> Remember (50)
    for item in RECALL_REMEMBER:
        user_msg, recall_args, recall_result, remember_args, remember_result, final = item
        chain = [
            ("recall", recall_args, recall_result),
            ("remember", remember_args, remember_result),
        ]
        examples.append(make_example(user_msg, chain, final, "recall_remember_chain"))

    # Cat 2: Recall -> Relate (30)
    for item in RECALL_RELATE:
        user_msg, recall_args, recall_result, relate_args, final = item
        chain = [
            ("recall", recall_args, recall_result),
            ("relate_entities", relate_args, {"status": "linked", "id": f"rel_{random.randint(100,999)}"}),
        ]
        examples.append(make_example(user_msg, chain, final, "recall_relate_chain"))

    # Cat 3: Forget flows - topic (15)
    for topic_item in FORGET_TOPIC_EXAMPLES:
        user_msg, topic, keywords, final = topic_item
        chain = [
            ("forget_topic", {"topic": topic, "keywords": keywords},
             {"deleted": random.randint(3, 18), "topic": topic}),
        ]
        examples.append(make_example(user_msg, chain, final, "forget_topic"))

    # Cat 3: Forget flows - single memory via recall (25)
    for item in FORGET_SINGLE_EXAMPLES:
        user_msg, recall_query, recall_result, rid, reason, final = item
        chain = [
            ("recall", {"query": recall_query}, recall_result),
            ("forget_memory", {"rid": rid, "reason": reason}, {"status": "deleted", "rid": rid}),
        ]
        examples.append(make_example(user_msg, chain, final, "forget_single_memory"))

    # Cat 4: Memory stats (5)
    for item in MEMORY_STATS_EXAMPLES:
        user_msg, stats_result, final = item
        chain = [("memory_stats", {}, stats_result)]
        examples.append(make_example(user_msg, chain, final, "memory_stats"))

    # Cat 4: Review memories (5)
    for item in REVIEW_MEMORIES_EXAMPLES:
        user_msg, review_args, review_result, final = item
        chain = [("review_memories", review_args, review_result)]
        examples.append(make_example(user_msg, chain, final, "review_memories"))

    # Cat 4: Purge system noise (5)
    for item in PURGE_EXAMPLES:
        user_msg, purge_args, purge_result, final = item
        chain = [("purge_system_noise", purge_args, purge_result)]
        examples.append(make_example(user_msg, chain, final, "purge_system_noise"))

    # Cat 4: Correction chains - review → forget → remember (10)
    for item in CORRECTION_CHAIN_EXAMPLES:
        user_msg, review_args, review_result, forget_rid, forget_reason, remember_args, remember_result, final = item
        chain = [
            ("review_memories", review_args, review_result),
            ("forget_memory", {"rid": forget_rid, "reason": forget_reason}, {"status": "deleted", "rid": forget_rid}),
            ("remember", remember_args, remember_result),
        ]
        examples.append(make_example(user_msg, chain, final, "correction_chain"))

    # Cat 4: Memory stats + review combo (15 extra hygiene examples)
    _combo_hygiene = [
        ("How's my memory organized? Show me the work ones.",
         {"total_memories": 180, "by_domain": {"work": 65, "personal": 40, "tech": 25, "health": 20, "family": 15, "system": 15}, "oldest": "2025-12-10", "newest": "2026-03-13"},
         {"domain": "work", "sort": "newest", "page": 1},
         {"memories": [
             {"rid": "mem_h01", "text": "User shipped v2 release.", "importance": 0.75, "created": "2026-03-12"},
             {"rid": "mem_h02", "text": "User got promoted.", "importance": 0.9, "created": "2026-03-10"},
         ], "total": 65, "page": 1},
         "180 memories total, 65 in work. Recent work highlights: v2 release shipped and your promotion."),
        ("Give me stats then show family stuff.",
         {"total_memories": 210, "by_domain": {"work": 78, "personal": 48, "family": 25, "health": 22, "tech": 20, "system": 17}, "oldest": "2025-11-20", "newest": "2026-03-13"},
         {"domain": "family", "sort": "importance_desc", "page": 1},
         {"memories": [
             {"rid": "mem_h03", "text": "User's baby born, named Oliver.", "importance": 0.95, "created": "2026-03-08"},
             {"rid": "mem_h04", "text": "User's mother was hospitalized.", "importance": 0.9, "created": "2026-03-06"},
         ], "total": 25, "page": 1},
         "210 memories, 25 about family. Top family memories: Oliver being born and your mom's hospitalization."),
        ("Memory report, then show me health entries.",
         {"total_memories": 145, "by_domain": {"work": 50, "personal": 35, "health": 25, "tech": 18, "family": 12, "system": 5}, "oldest": "2026-01-01", "newest": "2026-03-13"},
         {"domain": "health", "sort": "importance_desc", "page": 1},
         {"memories": [
             {"rid": "mem_h05", "text": "User is allergic to shellfish.", "importance": 0.95, "created": "2026-01-10"},
             {"rid": "mem_h06", "text": "User started blood pressure medication.", "importance": 0.8, "created": "2026-03-08"},
         ], "total": 25, "page": 1},
         "145 memories, 25 health-related. Key health items: shellfish allergy and blood pressure medication."),
        ("Overview and then show me the tech stuff.",
         {"total_memories": 275, "by_domain": {"work": 95, "personal": 55, "tech": 45, "health": 30, "system": 25, "family": 15, "finance": 10}, "oldest": "2025-11-01", "newest": "2026-03-13"},
         {"domain": "tech", "sort": "newest", "page": 1},
         {"memories": [
             {"rid": "mem_h07", "text": "User set up Proxmox home server.", "importance": 0.6, "created": "2026-03-10"},
             {"rid": "mem_h08", "text": "User switched main desktop to Linux.", "importance": 0.6, "created": "2026-03-05"},
         ], "total": 45, "page": 1},
         "275 memories across 7 domains, 45 in tech. Latest tech: Proxmox home server and the Linux switch."),
        ("What's the memory situation? Also show me finance.",
         {"total_memories": 190, "by_domain": {"work": 70, "personal": 42, "tech": 28, "health": 20, "finance": 15, "family": 10, "system": 5}, "oldest": "2025-12-15", "newest": "2026-03-13"},
         {"domain": "finance", "sort": "importance_desc", "page": 1},
         {"memories": [
             {"rid": "mem_h09", "text": "User paid off student loans.", "importance": 0.9, "created": "2026-03-10"},
             {"rid": "mem_h10", "text": "User sold crypto, moved to index funds.", "importance": 0.7, "created": "2026-03-05"},
         ], "total": 15, "page": 1},
         "190 memories, 15 financial. Top finance items: student loans paid off and the crypto-to-index-funds switch."),
        ("Stats please, then show personal memories.",
         {"total_memories": 160, "by_domain": {"work": 55, "personal": 40, "tech": 22, "health": 18, "family": 15, "system": 10}, "oldest": "2026-01-05", "newest": "2026-03-13"},
         {"domain": "personal", "sort": "newest", "page": 1},
         {"memories": [
             {"rid": "mem_h11", "text": "User moved to new apartment.", "importance": 0.7, "created": "2026-03-10"},
             {"rid": "mem_h12", "text": "User deleted Twitter account.", "importance": 0.5, "created": "2026-03-08"},
         ], "total": 40, "page": 1},
         "160 memories, 40 personal. Recent personal: new apartment move and Twitter deletion."),
        ("Quick memory status, show education stuff.",
         {"total_memories": 130, "by_domain": {"work": 48, "personal": 30, "education": 18, "tech": 15, "health": 12, "family": 7}, "oldest": "2026-01-10", "newest": "2026-03-13"},
         {"domain": "education", "sort": "newest", "page": 1},
         {"memories": [
             {"rid": "mem_h13", "text": "User enrolled in master's at Stanford.", "importance": 0.85, "created": "2026-03-10"},
             {"rid": "mem_h14", "text": "User completed thesis.", "importance": 0.95, "created": "2026-03-05"},
         ], "total": 18, "page": 1},
         "130 memories, 18 about education. Highlights: Stanford master's enrollment and thesis completion."),
        ("Memory check, then hobbies.",
         {"total_memories": 170, "by_domain": {"work": 60, "personal": 38, "hobbies": 22, "tech": 20, "health": 18, "family": 12}, "oldest": "2026-01-01", "newest": "2026-03-13"},
         {"domain": "hobbies", "sort": "importance_desc", "page": 1},
         {"memories": [
             {"rid": "mem_h15", "text": "User decided to learn piano.", "importance": 0.6, "created": "2026-03-10"},
             {"rid": "mem_h16", "text": "User learned Thai cooking.", "importance": 0.45, "created": "2026-03-08"},
         ], "total": 22, "page": 1},
         "170 memories, 22 in hobbies. You've been picking up piano and Thai cooking recently."),
        ("How many memories do you have? Show me the system ones.",
         {"total_memories": 220, "by_domain": {"work": 80, "personal": 45, "system": 35, "tech": 25, "health": 20, "family": 15}, "oldest": "2025-12-01", "newest": "2026-03-13"},
         {"domain": "system", "sort": "oldest", "page": 1},
         {"memories": [
             {"rid": "mem_h17", "text": "CPU usage spike detected on host.", "importance": 0.2, "created": "2025-12-15"},
             {"rid": "mem_h18", "text": "Process restart logged.", "importance": 0.15, "created": "2025-12-20"},
         ], "total": 35, "page": 1},
         "220 memories total, 35 system entries. Oldest system memories are from December. Could use a cleanup."),
        ("Summary and then newest memories overall.",
         {"total_memories": 200, "by_domain": {"work": 72, "personal": 45, "tech": 30, "health": 22, "family": 18, "system": 13}, "oldest": "2025-12-05", "newest": "2026-03-13"},
         {"domain": "work", "sort": "newest", "page": 1},  # show newest work as proxy
         {"memories": [
             {"rid": "mem_h19", "text": "User working on Kubernetes migration.", "importance": 0.7, "created": "2026-03-13"},
             {"rid": "mem_h20", "text": "User shipped v2 release.", "importance": 0.75, "created": "2026-03-12"},
         ], "total": 72, "page": 1},
         "200 memories. Latest activity: Kubernetes migration work today and the v2 release yesterday."),
        ("Memory overview, then show anything about relationships.",
         {"total_memories": 185, "by_domain": {"work": 68, "personal": 50, "family": 20, "health": 18, "tech": 15, "system": 14}, "oldest": "2025-12-10", "newest": "2026-03-13"},
         {"domain": "personal", "sort": "importance_desc", "page": 1},
         {"memories": [
             {"rid": "mem_h21", "text": "User got engaged.", "importance": 0.95, "created": "2026-03-10"},
             {"rid": "mem_h22", "text": "User adopted cat Luna.", "importance": 0.7, "created": "2026-03-05"},
         ], "total": 50, "page": 1},
         "185 memories. Top personal items: your engagement and Luna's adoption. Those are the big relationship moments."),
        ("How's memory health? And the oldest stuff I should clean up?",
         {"total_memories": 310, "by_domain": {"work": 110, "personal": 60, "system": 55, "tech": 35, "health": 25, "family": 15, "finance": 10}, "oldest": "2025-10-15", "newest": "2026-03-13"},
         {"domain": "system", "sort": "oldest", "page": 1},
         {"memories": [
             {"rid": "mem_h23", "text": "System boot logged.", "importance": 0.1, "created": "2025-10-15"},
             {"rid": "mem_h24", "text": "Network latency spike.", "importance": 0.15, "created": "2025-10-20"},
         ], "total": 55, "page": 1},
         "310 memories, 55 system entries going back to October 2025. Those system memories are prime cleanup candidates."),
        ("Memory stats, then show what you know about my routines.",
         {"total_memories": 175, "by_domain": {"work": 62, "personal": 38, "health": 25, "tech": 20, "family": 16, "routine": 14}, "oldest": "2025-12-20", "newest": "2026-03-13"},
         {"domain": "routine", "sort": "importance_desc", "page": 1},
         {"memories": [
             {"rid": "mem_h25", "text": "User wakes up around 7 AM.", "importance": 0.6, "created": "2026-01-05"},
             {"rid": "mem_h26", "text": "User deploys on Thursdays.", "importance": 0.6, "created": "2026-02-15"},
         ], "total": 14, "page": 1},
         "175 memories, 14 about routines. Key routines: 7 AM wakeups and Thursday deploys."),
        ("Total count and show me the least important stuff.",
         {"total_memories": 240, "by_domain": {"work": 85, "personal": 50, "tech": 35, "system": 30, "health": 20, "family": 12, "hobbies": 8}, "oldest": "2025-11-10", "newest": "2026-03-13"},
         {"domain": "system", "sort": "oldest", "page": 1},
         {"memories": [
             {"rid": "mem_h27", "text": "CPU temperature reading: 45C.", "importance": 0.1, "created": "2025-11-10"},
             {"rid": "mem_h28", "text": "Disk space check: 72% used.", "importance": 0.1, "created": "2025-11-15"},
         ], "total": 30, "page": 1},
         "240 memories. Lowest value: system telemetry from November. Temperature readings and disk checks. Want me to purge those?"),
        ("Quick count, then show recent finance memories.",
         {"total_memories": 155, "by_domain": {"work": 52, "personal": 35, "tech": 22, "health": 18, "finance": 16, "family": 12}, "oldest": "2026-01-01", "newest": "2026-03-13"},
         {"domain": "finance", "sort": "newest", "page": 1},
         {"memories": [
             {"rid": "mem_h29", "text": "User paid off student loans.", "importance": 0.9, "created": "2026-03-10"},
             {"rid": "mem_h30", "text": "Landlord raising rent by 20%.", "importance": 0.75, "created": "2026-03-08"},
         ], "total": 16, "page": 1},
         "155 memories, 16 financial. Recent: student loans paid off and the rent increase. Mixed bag financially."),
    ]

    for item in _combo_hygiene:
        user_msg, stats_result, review_args, review_result, final = item
        chain = [
            ("memory_stats", {}, stats_result),
            ("review_memories", review_args, review_result),
        ]
        examples.append(make_example(user_msg, chain, final, "stats_review_combo"))

    # Cat 5: Fix history (10)
    for item in FIX_HISTORY_EXAMPLES:
        user_msg, topic, result, final = item
        chain = [("search_fix_history", {"topic": topic}, result)]
        examples.append(make_example(user_msg, chain, final, "search_fix_history"))

    # Cat 5: Timeframe queries (10)
    for item in TIMEFRAME_EXAMPLES:
        user_msg, query, days_ago, result, final = item
        chain = [("search_by_timeframe", {"query": query, "days_ago": days_ago}, result)]
        examples.append(make_example(user_msg, chain, final, "search_by_timeframe"))

    # Cat 6: Introspection (20)
    for item in INTROSPECT_EXAMPLES:
        user_msg, query, result, final = item
        chain = [("introspect", {"query": query}, result)]
        examples.append(make_example(user_msg, chain, final, "introspection"))

    return examples


def main():
    examples = build_all()
    assert len(examples) == 200, f"Expected 200 examples, got {len(examples)}"

    with open(OUT, "w", encoding="utf-8") as f:
        for ex in examples:
            f.write(json.dumps(ex, ensure_ascii=False) + "\n")

    print(f"Wrote {len(examples)} examples to {OUT}")

    # Verify
    with open(OUT, "r", encoding="utf-8") as f:
        lines = f.readlines()
    print(f"Verification: {len(lines)} lines in output file")

    # Verify each line is valid JSON
    for i, line in enumerate(lines):
        try:
            obj = json.loads(line)
            assert "conversations" in obj
            assert "metadata" in obj
        except Exception as e:
            print(f"ERROR on line {i+1}: {e}")
            return

    # Category breakdown
    from collections import Counter
    cats = Counter()
    for line in lines:
        obj = json.loads(line)
        cats[obj["metadata"]["scenario_type"]] += 1
    print("Category breakdown:")
    for cat, count in sorted(cats.items()):
        print(f"  {cat}: {count}")


if __name__ == "__main__":
    main()
