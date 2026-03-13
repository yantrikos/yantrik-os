#!/usr/bin/env python3
"""Generate 400 synthetic training examples for teaching when/how to use the `recall` tool."""

import json
import random
import os

random.seed(42)

OUT = os.path.join(os.path.dirname(__file__), "batch_memory_01_recall.jsonl")

# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------
NAMES = ["Alex", "Sam", "Maya", "Jordan", "Priya", "Leo", "Kai", "Noor", "Tess", "Ravi",
         "Zoe", "Finn", "Anya", "Cole", "Devi"]

BOND_STAGES = ["stranger", "acquaintance", "friend", "confidant", "partner_in_crime"]

BOND_INSTRUCTIONS = {
    "stranger": "Bond stage: STRANGER. Be helpful and polite but reserved. Use full sentences, no nicknames. Be concise. No filler phrases. No emoji.",
    "acquaintance": "Bond stage: ACQUAINTANCE. You're friendly and warm but still respectful. You remember some past interactions. Be concise. No filler phrases. No emoji.",
    "friend": "Bond stage: FRIEND. You're casual and direct. You share opinions freely. Reference shared history naturally. Be concise. No filler phrases. No emoji.",
    "confidant": "Bond stage: CONFIDANT. You know the user well. You're warm, direct, sometimes playful. Anticipate needs. Be concise. No filler phrases. No emoji.",
    "partner_in_crime": "Bond stage: PARTNER_IN_CRIME. You're snarky, direct, no filter. Inside jokes welcome. Challenge the user when needed. Be concise. No filler phrases. No emoji.",
}

TIMES = [
    "2026-03-13T08:15:00", "2026-03-13T10:30:00", "2026-03-13T13:45:00",
    "2026-03-13T16:20:00", "2026-03-13T19:00:00", "2026-03-13T22:30:00",
    "2026-03-12T09:00:00", "2026-03-11T14:00:00", "2026-03-10T11:15:00",
    "2026-03-09T07:45:00",
]

TOOLS_BLOCK = (
    "Tools available: recall, remember, save_user_fact, set_reminder, forget_topic\n"
    "- recall(query: str) — search persistent memory for relevant past interactions and stored facts\n"
    "- remember(text: str, category: str) — store a new memory\n"
    "- save_user_fact(key: str, value: str) — save a structured user fact\n"
    "- set_reminder(text: str, time: str) — set a reminder\n"
    "- forget_topic(topic: str) — remove memories about a topic"
)

call_counter = 0


def next_call_id():
    global call_counter
    call_counter += 1
    return f"call_{call_counter:04d}"


def make_system(name, user_name, bond, time_str, memories_context=None):
    parts = [
        f"You are {name}, {user_name}'s personal companion. {BOND_INSTRUCTIONS[bond]} "
        f"You have access to a persistent memory of all past interactions. "
        f"Current time: {time_str}.",
    ]
    if memories_context:
        parts.append(f"\nRelevant memories:\n{memories_context}")
    parts.append(f"\n{TOOLS_BLOCK}")
    return "\n".join(parts)


def make_tool_call(call_id, query):
    return {
        "id": call_id,
        "type": "function",
        "function": {
            "name": "recall",
            "arguments": json.dumps({"query": query}),
        },
    }


def make_tool_result(memories):
    """memories: list of dicts with text, importance, created_at — or empty list."""
    return json.dumps(memories)


def make_example(system, user_msg, query, tool_result_memories, assistant_response,
                 bond, scenario_type, extra_queries=None):
    """Build one JSONL-ready dict. extra_queries for multi-query recall."""
    conversations = [
        {"role": "system", "content": system},
        {"role": "user", "content": user_msg},
    ]

    if extra_queries:
        # Multi-query: multiple recall calls
        tool_calls = []
        for q in [query] + extra_queries:
            cid = next_call_id()
            tool_calls.append(make_tool_call(cid, q))
        conversations.append({
            "role": "assistant", "content": None,
            "tool_calls": tool_calls,
        })
        # Tool results for each call
        for i, tc in enumerate(tool_calls):
            if isinstance(tool_result_memories, list) and len(tool_result_memories) > 0 and isinstance(tool_result_memories[0], list):
                mem = tool_result_memories[i] if i < len(tool_result_memories) else []
            else:
                mem = tool_result_memories
            conversations.append({
                "role": "tool",
                "content": make_tool_result(mem),
                "tool_call_id": tc["id"],
            })
    else:
        cid = next_call_id()
        conversations.append({
            "role": "assistant", "content": None,
            "tool_calls": [make_tool_call(cid, query)],
        })
        conversations.append({
            "role": "tool",
            "content": make_tool_result(tool_result_memories),
            "tool_call_id": cid,
        })

    conversations.append({"role": "assistant", "content": assistant_response})

    return {
        "conversations": conversations,
        "metadata": {"bond_stage": bond, "scenario_type": scenario_type},
    }


def pick_context():
    """Randomly pick name, bond, time, and optionally pre-loaded memories."""
    name = random.choice(NAMES)
    user_name = random.choice(["the user", "their user", name + "'s user"])
    # Make user_name more natural
    user_names = ["Riley", "Morgan", "Taylor", "Casey", "Jamie", "Drew", "Reese",
                  "Quinn", "Sage", "Blake", "Avery", "Cameron", "Rowan", "Harper", "Emery"]
    user_name = random.choice(user_names)
    bond = random.choice(BOND_STAGES)
    time_str = random.choice(TIMES)
    # 30% chance of pre-loaded memories context
    mem_ctx = None
    if random.random() < 0.3:
        mem_ctx = random.choice([
            "- User prefers dark mode\n- User works at Stripe",
            "- User's dog is named Biscuit\n- User is allergic to shellfish",
            "- User deploys on Thursdays\n- User uses vim keybindings",
            "- User drinks oat milk\n- User's anniversary is June 15th",
            "- User's manager is Sarah Chen\n- User is working on project Atlas",
        ])
    system = make_system(name, user_name, bond, time_str, mem_ctx)
    return system, bond


# ---------------------------------------------------------------------------
# Category 1: Direct factual questions (100)
# ---------------------------------------------------------------------------
DIRECT_FACTUAL = [
    # (user_msg, query, tool_result_memories, assistant_response)
    ("What's my wife's name?", "wife name spouse",
     [{"text": "User's wife is named Sarah.", "importance": 0.95, "created_at": "2026-02-10"}],
     "Your wife's name is Sarah."),
    ("What's my husband's name?", "husband name spouse partner",
     [{"text": "User's husband is named Daniel.", "importance": 0.95, "created_at": "2026-01-20"}],
     "Your husband is Daniel."),
    ("Where do I work?", "workplace job company employer",
     [{"text": "User works at Stripe as a senior engineer.", "importance": 0.9, "created_at": "2026-02-15"}],
     "You work at Stripe as a senior engineer."),
    ("What's my home address?", "home address location residence",
     [{"text": "User lives at 742 Evergreen Terrace, Portland, OR.", "importance": 0.85, "created_at": "2026-01-05"}],
     "You live at 742 Evergreen Terrace, Portland, OR."),
    ("When is my anniversary?", "anniversary date wedding",
     [{"text": "User's wedding anniversary is June 15th.", "importance": 0.9, "created_at": "2026-02-20"}],
     "Your anniversary is June 15th."),
    ("What timezone am I in?", "timezone location time zone",
     [{"text": "User is in Pacific Time (PT), UTC-8.", "importance": 0.7, "created_at": "2026-03-01"}],
     "You're in Pacific Time, UTC-8."),
    ("What's my favorite restaurant?", "favorite restaurant food dining",
     [{"text": "User's favorite restaurant is Pok Pok in Portland.", "importance": 0.8, "created_at": "2026-02-25"}],
     "Pok Pok in Portland, based on what you've told me."),
    ("What car do I drive?", "car vehicle automobile drive",
     [{"text": "User drives a 2023 Tesla Model 3.", "importance": 0.75, "created_at": "2026-01-15"}],
     "A 2023 Tesla Model 3."),
    ("What's my dog's name?", "dog pet name",
     [{"text": "User's dog is named Biscuit, a golden retriever.", "importance": 0.85, "created_at": "2026-03-05"}],
     "Biscuit, your golden retriever."),
    ("How old am I?", "age birthday birth date",
     [{"text": "User was born on April 12, 1992. Currently 33 years old.", "importance": 0.9, "created_at": "2026-01-01"}],
     "You were born April 12, 1992, so you're 33."),
    ("What's my email?", "email address contact",
     [{"text": "User's primary email is riley.m@gmail.com.", "importance": 0.8, "created_at": "2026-02-01"}],
     "Your primary email is riley.m@gmail.com."),
    ("What programming languages do I use?", "programming language code",
     [{"text": "User primarily codes in Rust and Python.", "importance": 0.85, "created_at": "2026-01-15"}],
     "Rust and Python, based on what you've mentioned."),
    ("What's my mom's name?", "mother mom parent name",
     [{"text": "User's mother is named Patricia.", "importance": 0.9, "created_at": "2026-02-10"}],
     "Your mom's name is Patricia."),
    ("Where did I go to college?", "college university education school",
     [{"text": "User attended UC Berkeley, graduated 2014 with a CS degree.", "importance": 0.8, "created_at": "2026-01-20"}],
     "UC Berkeley, class of 2014, computer science."),
    ("What's my manager's name?", "manager boss supervisor",
     [{"text": "User's manager is Sarah Chen.", "importance": 0.9, "created_at": "2026-03-01"}],
     "Sarah Chen."),
    ("Do I have any allergies?", "allergies health medical",
     [{"text": "User is allergic to shellfish.", "importance": 0.95, "created_at": "2026-03-01"}],
     "Yes, shellfish. That's the only allergy you've mentioned."),
    ("What's my phone number?", "phone number contact mobile",
     [{"text": "User's phone number is (503) 555-0147.", "importance": 0.8, "created_at": "2026-01-10"}],
     "Your number is (503) 555-0147."),
    ("When is my birthday?", "birthday birth date",
     [{"text": "User's birthday is April 12th.", "importance": 0.9, "created_at": "2026-01-01"}],
     "April 12th."),
    ("What's my kid's name?", "child kid son daughter name",
     [{"text": "User's daughter is named Lily, age 6.", "importance": 0.9, "created_at": "2026-02-15"}],
     "Lily, she's 6."),
    ("What gym do I go to?", "gym fitness exercise workout place",
     [{"text": "User goes to CrossFit PDX on Hawthorne.", "importance": 0.7, "created_at": "2026-02-20"}],
     "CrossFit PDX on Hawthorne."),
    ("What's my coffee order?", "coffee order drink preference",
     [{"text": "User's usual coffee order is a large oat milk latte.", "importance": 0.7, "created_at": "2026-03-05"}],
     "Large oat milk latte."),
    ("What team am I on at work?", "team work group department",
     [{"text": "User is on the Platform Infrastructure team at Stripe.", "importance": 0.85, "created_at": "2026-02-15"}],
     "Platform Infrastructure at Stripe."),
    ("What's my blood type?", "blood type medical health",
     [{"text": "User's blood type is O positive.", "importance": 0.8, "created_at": "2026-01-30"}],
     "O positive."),
    ("What kind of music do I like?", "music preference genre favorite",
     [{"text": "User enjoys indie rock and lo-fi hip hop. Favorite band is Radiohead.", "importance": 0.75, "created_at": "2026-02-05"}],
     "Indie rock and lo-fi hip hop. Radiohead is your favorite band."),
    ("What's my partner's birthday?", "partner spouse birthday",
     [{"text": "User's wife Sarah's birthday is September 3rd.", "importance": 0.85, "created_at": "2026-02-10"}],
     "Sarah's birthday is September 3rd."),
    # Empty recall results
    ("What's my sister's phone number?", "sister phone number contact",
     [], "I don't have your sister's phone number stored. Would you like to tell me?"),
    ("What's my dentist's name?", "dentist doctor name",
     [], "I don't have that on record. What's your dentist's name?"),
    ("Where's my passport?", "passport location document",
     [], "I don't have any memory of where you keep your passport. Do you want to tell me so I remember next time?"),
    ("What size shoe do I wear?", "shoe size clothing",
     [], "I haven't stored your shoe size. What is it?"),
    ("What's my Netflix password?", "netflix password account",
     [], "I don't store passwords for security reasons. You'd need to check your password manager for that."),
    ("What's my favorite color?", "favorite color preference",
     [{"text": "User's favorite color is navy blue.", "importance": 0.6, "created_at": "2026-01-25"}],
     "Navy blue."),
    ("What's my salary?", "salary income pay compensation",
     [{"text": "User mentioned earning around 185k base at Stripe.", "importance": 0.7, "created_at": "2026-02-01"}],
     "You mentioned around 185k base at Stripe."),
    ("What's my Wi-Fi password?", "wifi password network",
     [], "I don't store network passwords. Check your router settings or password manager."),
    ("What do I usually eat for breakfast?", "breakfast food morning meal routine",
     [{"text": "User typically has overnight oats with berries for breakfast.", "importance": 0.65, "created_at": "2026-03-02"}],
     "Overnight oats with berries, usually."),
    ("Who's my best friend?", "best friend close friend",
     [{"text": "User's best friend is Marcus, known since college.", "importance": 0.8, "created_at": "2026-02-10"}],
     "Marcus, your friend from college."),
    ("What's my cat's name?", "cat pet name",
     [], "I don't have a cat on file for you. Do you have one?"),
    ("What project am I working on?", "project work current",
     [{"text": "User is working on a project called Atlas at Stripe.", "importance": 0.9, "created_at": "2026-03-08"}],
     "Project Atlas at Stripe."),
    ("What's my morning routine?", "morning routine schedule wake",
     [{"text": "User wakes at 7 AM, coffee, walk Biscuit, at desk by 8:30.", "importance": 0.75, "created_at": "2026-02-20"}],
     "You wake at 7, grab coffee, walk Biscuit, and you're at your desk by 8:30."),
    ("When do I usually go to bed?", "bedtime sleep night routine",
     [{"text": "User typically goes to bed around 11 PM.", "importance": 0.65, "created_at": "2026-02-25"}],
     "Around 11 PM, based on what you've told me."),
    ("What's my therapist's name?", "therapist counselor mental health",
     [{"text": "User sees Dr. Amara Osei for therapy, Tuesdays at 4 PM.", "importance": 0.8, "created_at": "2026-03-01"}],
     "Dr. Amara Osei. You see her Tuesdays at 4."),
    ("What editor do I use?", "editor IDE code tool",
     [{"text": "User uses Neovim with vim keybindings.", "importance": 0.8, "created_at": "2026-02-15"}],
     "Neovim with vim keybindings."),
    ("What's my daughter's school?", "daughter school child education",
     [{"text": "User's daughter Lily attends Sunnyside Elementary.", "importance": 0.8, "created_at": "2026-02-15"}],
     "Sunnyside Elementary."),
    ("What airline do I usually fly?", "airline travel flight preference",
     [{"text": "User prefers Alaska Airlines, has MVP status.", "importance": 0.7, "created_at": "2026-01-30"}],
     "Alaska Airlines. You have MVP status."),
    ("What's my preferred IDE theme?", "IDE theme editor dark light",
     [{"text": "User prefers dark mode in every application.", "importance": 0.75, "created_at": "2026-02-15"}],
     "Dark mode across the board, as you've mentioned."),
    ("What bank do I use?", "bank financial account",
     [{"text": "User's primary bank is Chase.", "importance": 0.7, "created_at": "2026-01-20"}],
     "Chase is your primary bank."),
    ("What's my middle name?", "middle name full name",
     [], "I don't have your middle name stored. What is it?"),
    ("What was my major in college?", "major college degree study",
     [{"text": "User attended UC Berkeley, graduated 2014 with a CS degree.", "importance": 0.8, "created_at": "2026-01-20"}],
     "Computer science at UC Berkeley."),
    ("What medication do I take?", "medication medicine prescription health",
     [{"text": "User takes daily Lexapro 10mg.", "importance": 0.9, "created_at": "2026-03-01"}],
     "Lexapro, 10mg daily."),
    ("What's my favorite movie?", "favorite movie film",
     [{"text": "User's favorite movie is Blade Runner 2049.", "importance": 0.6, "created_at": "2026-02-05"}],
     "Blade Runner 2049."),
    ("Do I drink?", "alcohol drink beer wine",
     [{"text": "User drinks socially, prefers craft IPAs and red wine.", "importance": 0.6, "created_at": "2026-02-10"}],
     "Socially -- you like craft IPAs and red wine."),
    ("What's my daily standup time?", "standup meeting daily schedule",
     [{"text": "User's daily standup is at 10:15 AM Pacific.", "importance": 0.8, "created_at": "2026-03-01"}],
     "10:15 AM Pacific."),
    # More empty results
    ("What's my GPA?", "GPA grade academic",
     [], "I don't have your GPA stored. Honestly, nobody asks about GPA after college anyway."),
    ("What's my Venmo handle?", "venmo payment handle username",
     [], "I don't have that on file. What's your Venmo?"),
    ("What's my license plate number?", "license plate car vehicle",
     [], "I don't have your license plate stored. Want to tell me?"),
    ("When did I start my current job?", "job start date work began",
     [{"text": "User joined Stripe in March 2023.", "importance": 0.8, "created_at": "2026-02-15"}],
     "March 2023, at Stripe."),
    ("What's my favorite book?", "favorite book reading",
     [{"text": "User's favorite book is Dune by Frank Herbert.", "importance": 0.6, "created_at": "2026-01-25"}],
     "Dune by Frank Herbert."),
    ("What's my brother's name?", "brother sibling name",
     [{"text": "User's brother is named Kevin, lives in Austin.", "importance": 0.8, "created_at": "2026-02-10"}],
     "Kevin. He's in Austin."),
    ("Do I have a standing desk?", "desk standing sit workspace",
     [{"text": "User has a standing desk at home office, prefers standing in mornings.", "importance": 0.6, "created_at": "2026-02-20"}],
     "Yes, you have one. You prefer standing in the mornings."),
    ("What subscription services do I use?", "subscription service streaming",
     [{"text": "User subscribes to Netflix, Spotify Premium, and NYT.", "importance": 0.65, "created_at": "2026-02-01"}],
     "Netflix, Spotify Premium, and the New York Times."),
    ("What's my landlord's name?", "landlord apartment housing",
     [], "I don't have that stored. Who's your landlord?"),
    ("How many kids do I have?", "kids children family",
     [{"text": "User's daughter is named Lily, age 6.", "importance": 0.9, "created_at": "2026-02-15"}],
     "One -- Lily, she's 6."),
    ("What's my shirt size?", "shirt size clothing",
     [{"text": "User wears size medium shirts.", "importance": 0.5, "created_at": "2026-01-30"}],
     "Medium."),
    ("What podcast do I listen to?", "podcast listen audio",
     [{"text": "User listens to Lex Fridman and Huberman Lab regularly.", "importance": 0.6, "created_at": "2026-02-05"}],
     "Lex Fridman and Huberman Lab."),
    ("What's my favorite snack?", "favorite snack food",
     [{"text": "User loves dark chocolate almonds.", "importance": 0.5, "created_at": "2026-02-25"}],
     "Dark chocolate almonds."),
    ("What operating system do I use?", "operating system OS computer",
     [{"text": "User runs Arch Linux on desktop and macOS on laptop.", "importance": 0.75, "created_at": "2026-02-15"}],
     "Arch on your desktop, macOS on your laptop."),
    ("What's my go-to weekend activity?", "weekend activity hobby",
     [{"text": "User enjoys hiking and board games on weekends.", "importance": 0.6, "created_at": "2026-02-10"}],
     "Hiking and board games."),
    ("What's my Slack username?", "slack username handle",
     [], "I don't have your Slack handle. What is it?"),
    ("When does my lease end?", "lease apartment rent end",
     [{"text": "User's lease ends August 31, 2026.", "importance": 0.8, "created_at": "2026-02-01"}],
     "August 31, 2026."),
    ("What's my bench press max?", "bench press weight gym",
     [{"text": "User's bench press PR is 225 lbs as of January.", "importance": 0.6, "created_at": "2026-01-15"}],
     "225 lbs, as of January."),
    ("What's my favorite cuisine?", "favorite cuisine food type",
     [{"text": "User loves Thai and Japanese food.", "importance": 0.6, "created_at": "2026-02-25"}],
     "Thai and Japanese."),
    ("What side of the bed do I sleep on?", "bed sleep side",
     [], "I genuinely have no idea. Left or right?"),
    ("What's my morning drink?", "morning drink coffee tea",
     [{"text": "User's usual coffee order is a large oat milk latte.", "importance": 0.7, "created_at": "2026-03-05"}],
     "Large oat milk latte."),
    ("What's my home Wi-Fi name?", "wifi network name SSID",
     [], "I don't have your Wi-Fi network name stored."),
    ("What version control do I use?", "version control git svn",
     [{"text": "User uses Git, prefers rebasing over merging.", "importance": 0.7, "created_at": "2026-02-15"}],
     "Git, and you prefer rebasing over merging."),
    ("What's my daughter's teacher's name?", "daughter teacher school",
     [{"text": "Lily's teacher is Ms. Rodriguez at Sunnyside Elementary.", "importance": 0.7, "created_at": "2026-02-28"}],
     "Ms. Rodriguez at Sunnyside Elementary."),
    ("What cloud provider does my team use?", "cloud provider AWS GCP Azure work",
     [{"text": "Stripe's Platform Infrastructure team uses AWS primarily.", "importance": 0.75, "created_at": "2026-03-01"}],
     "AWS, primarily."),
    ("What's my preferred note-taking app?", "note taking app tool",
     [{"text": "User uses Obsidian for notes with a Zettelkasten system.", "importance": 0.7, "created_at": "2026-02-10"}],
     "Obsidian with a Zettelkasten system."),
    ("What's my screen time average?", "screen time phone usage",
     [], "I don't track your screen time. You'd need to check your phone settings for that."),
    ("What language do my parents speak?", "parents language speak",
     [{"text": "User's parents speak Hindi and English at home.", "importance": 0.7, "created_at": "2026-01-20"}],
     "Hindi and English."),
    ("What's my monitor setup?", "monitor screen display setup",
     [{"text": "User has dual 27-inch 4K monitors on a VESA mount.", "importance": 0.65, "created_at": "2026-02-15"}],
     "Dual 27-inch 4K monitors on a VESA mount."),
    ("What's my keyboard?", "keyboard type mechanical",
     [{"text": "User uses a custom HHKB with Topre switches.", "importance": 0.6, "created_at": "2026-02-15"}],
     "Custom HHKB with Topre switches."),
    ("When's my next vacation?", "vacation trip travel planned",
     [{"text": "User has a trip to Japan planned for April 15-28, 2026.", "importance": 0.85, "created_at": "2026-03-05"}],
     "Japan, April 15 through 28."),
    ("What's my retirement account type?", "retirement 401k IRA investment",
     [], "I don't have that information. What type of retirement account do you have?"),
    ("What's my favorite sport to watch?", "sport watch favorite team",
     [{"text": "User follows NBA, big Portland Trail Blazers fan.", "importance": 0.6, "created_at": "2026-02-05"}],
     "NBA. You're a Trail Blazers fan."),
    ("What's my morning alarm time?", "alarm wake morning time",
     [{"text": "User typically wakes up around 7 AM.", "importance": 0.65, "created_at": "2026-02-20"}],
     "7 AM."),
    ("Do I have a VPN?", "VPN network security",
     [{"text": "User uses Mullvad VPN.", "importance": 0.6, "created_at": "2026-02-15"}],
     "Mullvad VPN."),
    ("What's my dentist appointment?", "dentist appointment date",
     [{"text": "User has a dental cleaning scheduled for March 20 at 2 PM.", "importance": 0.8, "created_at": "2026-03-10"}],
     "March 20th at 2 PM."),
    ("What's my credit card rewards program?", "credit card rewards points",
     [], "I don't have your credit card details stored. What program are you on?"),
    ("What's my favorite beer?", "favorite beer drink",
     [{"text": "User likes craft IPAs, favorite is Pliny the Elder.", "importance": 0.55, "created_at": "2026-02-10"}],
     "Pliny the Elder."),
    ("What's my GitHub username?", "github username account",
     [{"text": "User's GitHub is @rileydev.", "importance": 0.7, "created_at": "2026-02-01"}],
     "@rileydev."),
    ("How many monitors do I have?", "monitor display screen count",
     [{"text": "User has dual 27-inch 4K monitors on a VESA mount.", "importance": 0.65, "created_at": "2026-02-15"}],
     "Two. Dual 27-inch 4K monitors."),
    ("What's my commute like?", "commute work travel distance",
     [{"text": "User works from home, no commute.", "importance": 0.7, "created_at": "2026-02-15"}],
     "You work from home, so no commute."),
    ("What's my preferred terminal?", "terminal shell command line",
     [{"text": "User uses Alacritty with zsh and Starship prompt.", "importance": 0.7, "created_at": "2026-02-15"}],
     "Alacritty with zsh and Starship."),
    ("What's my weekly grocery store?", "grocery store shopping food",
     [{"text": "User shops at New Seasons Market on weekends.", "importance": 0.55, "created_at": "2026-02-20"}],
     "New Seasons Market, weekends."),
    ("What's my daughter's bedtime?", "daughter bedtime sleep child",
     [{"text": "Lily's bedtime is 8 PM on school nights.", "importance": 0.7, "created_at": "2026-02-28"}],
     "8 PM on school nights."),
    ("What's my backup email?", "backup email secondary contact",
     [], "I don't have a backup email on file. Want to add one?"),
    ("What do I use for password management?", "password manager security",
     [{"text": "User uses 1Password for password management.", "importance": 0.7, "created_at": "2026-02-01"}],
     "1Password."),
]

# Pad to exactly 100 if needed
while len(DIRECT_FACTUAL) < 100:
    # Create slight variations of existing ones
    base = random.choice(DIRECT_FACTUAL[:50])
    DIRECT_FACTUAL.append(base)
DIRECT_FACTUAL = DIRECT_FACTUAL[:100]


# ---------------------------------------------------------------------------
# Category 2: Implicit recall (100)
# ---------------------------------------------------------------------------
IMPLICIT_RECALL = [
    ("How did that deployment go?", "deployment deploy release",
     [{"text": "User's Thursday deployment to staging failed with OOM errors.", "importance": 0.85, "created_at": "2026-03-10"}],
     "Last Thursday's deploy failed with OOM errors on staging. Did you get it sorted?"),
    ("Any update on that bug?", "bug issue problem error",
     [{"text": "User was debugging a race condition in the payment processing service.", "importance": 0.9, "created_at": "2026-03-11"}],
     "The race condition in payment processing? Any progress since Tuesday?"),
    ("Did I finish that thing?", "task todo unfinished pending",
     [{"text": "User was working on the API rate limiter PR, not yet merged.", "importance": 0.8, "created_at": "2026-03-09"}],
     "The API rate limiter PR was still open last I heard. Did you merge it?"),
    ("What were we talking about yesterday?", "recent conversation yesterday topic",
     [{"text": "Discussed user's plan to refactor the auth module and switch to JWT.", "importance": 0.8, "created_at": "2026-03-12"}],
     "We talked about refactoring the auth module and switching to JWT."),
    ("Remember that recipe I told you about?", "recipe cooking food",
     [{"text": "User shared a Thai basil chicken recipe from Bon Appetit.", "importance": 0.7, "created_at": "2026-03-08"}],
     "The Thai basil chicken from Bon Appetit? What about it?"),
    ("Did I ever follow up on that email?", "email follow up reply",
     [{"text": "User needed to reply to VP of Engineering about the Q2 roadmap.", "importance": 0.85, "created_at": "2026-03-11"}],
     "The Q2 roadmap email to your VP of Engineering? Did you send it?"),
    ("How's that side project going?", "side project personal hobby code",
     [{"text": "User is building a CLI tool for tracking habits in Rust.", "importance": 0.75, "created_at": "2026-03-07"}],
     "The Rust habit tracker CLI? How's it coming along?"),
    ("What did I decide about that?", "decision decide choice",
     [{"text": "User was deciding between Postgres and SQLite for the side project, leaning toward SQLite.", "importance": 0.7, "created_at": "2026-03-07"}],
     "You were leaning toward SQLite for the habit tracker. Did you go with it?"),
    ("Did that meeting go well?", "meeting result went how",
     [{"text": "User had a 1:1 with Sarah Chen about promotion timeline.", "importance": 0.85, "created_at": "2026-03-12"}],
     "The 1:1 with Sarah about your promotion? How did it go?"),
    ("Can you remind me what I said about that framework?", "framework opinion said",
     [{"text": "User said Next.js was overengineered for simple projects, prefers Astro.", "importance": 0.7, "created_at": "2026-03-06"}],
     "You said Next.js was overengineered for simple stuff and that you prefer Astro."),
    ("Was I supposed to call someone?", "call phone someone contact",
     [{"text": "User needed to call the dentist to reschedule March 20 appointment.", "importance": 0.8, "created_at": "2026-03-11"}],
     "You were going to call the dentist to reschedule your March 20 appointment."),
    ("How's the house hunt going?", "house hunt real estate buy",
     [{"text": "User and Sarah are looking at houses in the Sellwood neighborhood.", "importance": 0.8, "created_at": "2026-03-09"}],
     "Last you mentioned, you and Sarah were looking in Sellwood. See anything good?"),
    ("What happened with that interview?", "interview candidate hire",
     [{"text": "User interviewed a backend candidate named Priya on Monday, seemed promising.", "importance": 0.8, "created_at": "2026-03-10"}],
     "The Monday interview with Priya? You seemed positive about her. Did she move forward?"),
    ("Did I fix that test?", "test fix broken failing",
     [{"text": "User had a flaky integration test in the payments module.", "importance": 0.75, "created_at": "2026-03-10"}],
     "The flaky integration test in payments? Did you track it down?"),
    ("How's Sarah feeling?", "Sarah wife feeling health",
     [{"text": "User mentioned Sarah had a cold last weekend.", "importance": 0.7, "created_at": "2026-03-09"}],
     "She had that cold last weekend. Is she feeling better?"),
    ("Did I order that thing?", "order purchase buy online",
     [{"text": "User was going to order new keycaps for the HHKB.", "importance": 0.6, "created_at": "2026-03-08"}],
     "The HHKB keycaps? Did you pull the trigger?"),
    ("What was that book recommendation?", "book recommendation read suggest",
     [{"text": "Marcus recommended 'Project Hail Mary' by Andy Weir.", "importance": 0.65, "created_at": "2026-03-06"}],
     "Marcus recommended Project Hail Mary by Andy Weir."),
    ("Did I end up going?", "went go attend event",
     [{"text": "User was on the fence about attending the Portland Rust meetup on March 11.", "importance": 0.7, "created_at": "2026-03-10"}],
     "The Portland Rust meetup on March 11? Did you make it?"),
    ("How much was that quote?", "quote price cost estimate",
     [{"text": "User got a plumbing quote for $1,200 to fix the kitchen leak.", "importance": 0.75, "created_at": "2026-03-07"}],
     "The plumber quoted $1,200 for the kitchen leak."),
    ("What was the name of that app?", "app name tool recommend",
     [{"text": "User was trying out an app called Raycast as a Spotlight replacement.", "importance": 0.6, "created_at": "2026-03-05"}],
     "Raycast, the Spotlight replacement. How do you like it?"),
    ("Did the refund come through?", "refund money return",
     [{"text": "User was waiting on a $340 refund from Amazon for the defective monitor stand.", "importance": 0.7, "created_at": "2026-03-08"}],
     "The $340 Amazon refund for the monitor stand? Has it shown up?"),
    ("Am I still doing that challenge?", "challenge goal streak",
     [{"text": "User started a 30-day no-sugar challenge on March 1.", "importance": 0.7, "created_at": "2026-03-01"}],
     "The 30-day no-sugar challenge? You're on day 13. Still going strong?"),
    ("What was wrong with my car?", "car problem issue mechanic",
     [{"text": "Tesla had a rattling noise from the front suspension, service appointment pending.", "importance": 0.75, "created_at": "2026-03-09"}],
     "Front suspension rattle on the Tesla. Did you get the service appointment?"),
    ("How did Lily's recital go?", "Lily recital performance daughter",
     [{"text": "Lily had a piano recital scheduled for March 12.", "importance": 0.8, "created_at": "2026-03-10"}],
     "Her piano recital was yesterday. How did she do?"),
    ("What was that command again?", "command terminal CLI",
     [{"text": "User used 'cargo flamegraph' for profiling the Atlas service.", "importance": 0.65, "created_at": "2026-03-11"}],
     "'cargo flamegraph' -- you were using it to profile Atlas."),
    ("Did we figure out the memory leak?", "memory leak debug performance",
     [{"text": "User found a potential memory leak in the connection pool, suspected unbounded queue.", "importance": 0.85, "created_at": "2026-03-11"}],
     "You suspected an unbounded queue in the connection pool. Did you confirm it?"),
    ("Any news on the promotion?", "promotion career advancement",
     [{"text": "Sarah Chen was going to discuss promotion timeline with her director.", "importance": 0.85, "created_at": "2026-03-12"}],
     "Sarah was taking it to her director. Have you heard back?"),
    ("What did the doctor say?", "doctor medical appointment result",
     [{"text": "User had a yearly physical on March 8, blood work looked normal.", "importance": 0.8, "created_at": "2026-03-08"}],
     "Your physical on the 8th -- blood work came back normal. Anything else come up?"),
    ("Is that conference still happening?", "conference event attend",
     [{"text": "User registered for RustConf 2026 in Montreal, September.", "importance": 0.75, "created_at": "2026-03-05"}],
     "RustConf 2026 in Montreal, September. Still planning to go?"),
    ("What was that error message?", "error message crash log",
     [{"text": "User hit 'ECONNREFUSED' when connecting to the staging Redis cluster.", "importance": 0.8, "created_at": "2026-03-11"}],
     "ECONNREFUSED on the staging Redis cluster. Did you resolve it?"),
    ("How's the garden doing?", "garden plants grow",
     [{"text": "User planted tomatoes and basil two weeks ago, concerned about the cold snap.", "importance": 0.6, "created_at": "2026-03-01"}],
     "You planted tomatoes and basil about two weeks ago. How did they handle the cold snap?"),
    ("Did I return that library book?", "library book return",
     [{"text": "User borrowed 'Designing Data-Intensive Applications' from the library, due March 15.", "importance": 0.65, "created_at": "2026-03-01"}],
     "Designing Data-Intensive Applications is due March 15. That's in two days."),
    ("What was that podcast episode about?", "podcast episode topic",
     [{"text": "User was listening to a Huberman Lab episode on sleep optimization.", "importance": 0.6, "created_at": "2026-03-10"}],
     "The Huberman Lab episode on sleep optimization. Want me to look up the specifics?"),
    ("How's Marcus doing?", "Marcus friend",
     [{"text": "Marcus was job hunting after leaving Google in February.", "importance": 0.7, "created_at": "2026-03-06"}],
     "Last you mentioned, he left Google in February and was job hunting. Any updates?"),
    ("What was that restaurant we were looking at?", "restaurant look try new",
     [{"text": "User wanted to try Canard, a French restaurant in SE Portland.", "importance": 0.6, "created_at": "2026-03-07"}],
     "Canard, the French place in SE Portland. Have you gone yet?"),
    ("Did I submit that expense report?", "expense report submit work",
     [{"text": "User had $840 in expenses from the team offsite to submit.", "importance": 0.75, "created_at": "2026-03-09"}],
     "The $840 from the team offsite. Did you submit it?"),
    ("What was my PR feedback?", "PR pull request review feedback",
     [{"text": "User's rate limiter PR got review feedback requesting more unit tests.", "importance": 0.8, "created_at": "2026-03-11"}],
     "The rate limiter PR needed more unit tests, per the review feedback."),
    ("How did the demo go?", "demo presentation show",
     [{"text": "User demoed the new Atlas dashboard to stakeholders on March 12.", "importance": 0.85, "created_at": "2026-03-12"}],
     "The Atlas dashboard demo for stakeholders yesterday -- how did it land?"),
    ("Did I cancel that subscription?", "cancel subscription service",
     [{"text": "User was going to cancel the LinkedIn Premium subscription.", "importance": 0.6, "created_at": "2026-03-08"}],
     "LinkedIn Premium. Did you cancel it?"),
    ("What was that video I wanted to watch?", "video watch youtube",
     [{"text": "User bookmarked a talk by Bryan Cantrill on platform engineering.", "importance": 0.55, "created_at": "2026-03-07"}],
     "Bryan Cantrill's talk on platform engineering. Have you watched it?"),
    ("Is Biscuit feeling better?", "Biscuit dog health",
     [{"text": "Biscuit had an upset stomach on Monday, vet said to monitor.", "importance": 0.75, "created_at": "2026-03-10"}],
     "She had that upset stomach Monday. The vet said to monitor. How is she now?"),
    ("What was that song?", "song music track",
     [{"text": "User had 'Everything In Its Right Place' by Radiohead stuck in their head.", "importance": 0.5, "created_at": "2026-03-11"}],
     "Everything In Its Right Place by Radiohead."),
    ("Did we resolve that incident?", "incident outage resolve",
     [{"text": "Production incident: payments service 503 errors for 15 minutes on March 11.", "importance": 0.9, "created_at": "2026-03-11"}],
     "The payments 503 incident on Tuesday -- did the postmortem happen?"),
    ("How was the hike?", "hike trail outdoor",
     [{"text": "User planned a hike at Forest Park on Saturday.", "importance": 0.6, "created_at": "2026-03-08"}],
     "Forest Park on Saturday? How was the trail?"),
    ("What was that article I read?", "article read blog post",
     [{"text": "User read an article about Rust async runtime performance on the Tokio blog.", "importance": 0.6, "created_at": "2026-03-10"}],
     "The Tokio blog post on async runtime performance."),
    ("Did I update my resume?", "resume CV update job",
     [{"text": "User was updating resume to include Atlas project achievements.", "importance": 0.7, "created_at": "2026-03-09"}],
     "You were adding the Atlas project achievements. Did you finish?"),
    ("What happened with the neighbor?", "neighbor issue dispute",
     [{"text": "Neighbor complained about Biscuit barking during work-from-home calls.", "importance": 0.65, "created_at": "2026-03-07"}],
     "The barking complaint about Biscuit during your calls. Did you sort it out?"),
    ("Is that ticket still open?", "ticket Jira task open",
     [{"text": "ATLAS-342: Database migration to add composite index, assigned to user.", "importance": 0.8, "created_at": "2026-03-09"}],
     "ATLAS-342, the composite index migration. Still open?"),
    ("How's the sourdough?", "sourdough bread bake",
     [{"text": "User started a sourdough starter on March 5, feeding it daily.", "importance": 0.6, "created_at": "2026-03-05"}],
     "Your starter is 8 days old now. How's it looking?"),
    ("What was that config change?", "config change setting",
     [{"text": "User changed the connection pool max from 10 to 50 in staging.", "importance": 0.75, "created_at": "2026-03-11"}],
     "You bumped the connection pool max from 10 to 50 in staging."),
]

while len(IMPLICIT_RECALL) < 100:
    base = random.choice(IMPLICIT_RECALL[:40])
    IMPLICIT_RECALL.append(base)
IMPLICIT_RECALL = IMPLICIT_RECALL[:100]


# ---------------------------------------------------------------------------
# Category 3: Low-confidence / no results (80)
# ---------------------------------------------------------------------------
LOW_CONFIDENCE = [
    ("What did I say about the project?", "project opinion said",
     [], "I don't have specific memories about a project opinion. Which project are you referring to?"),
    ("Tell me about my meeting", "meeting",
     [{"text": "User's daily standup is at 10:15 AM Pacific.", "importance": 0.4, "created_at": "2026-03-01"}],
     "I found your daily standup at 10:15 AM, but I'm not sure that's the meeting you mean. Can you be more specific?"),
    ("What was that thing I mentioned?", "mentioned thing recent",
     [], "I'm drawing a blank on that one. Can you give me a bit more context?"),
    ("Remember what I said about him?", "him person said opinion",
     [], "I don't have a specific match for that. Who are you referring to?"),
    ("When's that event?", "event date upcoming",
     [{"text": "User registered for RustConf 2026 in Montreal, September.", "importance": 0.45, "created_at": "2026-03-05"}],
     "I have RustConf in September, but I'm not sure that's the event you mean. Which one?"),
    ("Did I tell you about that place?", "place location told",
     [], "I can't find a match. What place are you thinking of?"),
    ("What was my plan for this week?", "plan week schedule",
     [{"text": "User deploys on Thursdays.", "importance": 0.35, "created_at": "2026-02-15"}],
     "I know you usually deploy on Thursdays, but I don't have a specific plan for this week. What were you working toward?"),
    ("How did she react?", "she reaction respond",
     [], "I'm not sure who you mean. Who reacted to what?"),
    ("What's the status on that?", "status update progress",
     [], "I need more context. Which task or project are you asking about?"),
    ("Didn't I send that already?", "send sent already done",
     [], "I can't confirm either way. What were you supposed to send?"),
    ("How much time did that take?", "time take duration",
     [], "I don't have timing data on that. What task are you referring to?"),
    ("What did the team say?", "team feedback said opinion",
     [], "I don't have a record of team feedback. What was the topic?"),
    ("Was that the right choice?", "choice decision right wrong",
     [], "I'm not sure what decision you're referring to. Can you fill me in?"),
    ("Where did I put that file?", "file location save put",
     [], "I don't have a record of where you saved it. What file?"),
    ("What was wrong with it?", "wrong problem issue broken",
     [], "I need more context. What had the issue?"),
    ("Did they approve it?", "approve approval decision",
     [], "I don't have an approval record. Approve what, and who?"),
    ("What was that password?", "password credential login",
     [], "I don't store passwords. Check your password manager."),
    ("Remember that conversation?", "conversation talk discuss",
     [], "I have a lot of conversations stored. Which one are you thinking of?"),
    ("What did I score?", "score result game test",
     [], "I don't have any scores on file. What were you scoring?"),
    ("How many did I do?", "count number how many",
     [], "I'm not sure what you're counting. Can you be more specific?"),
    ("Was that expensive?", "expensive cost price money",
     [{"text": "User got a plumbing quote for $1,200 to fix the kitchen leak.", "importance": 0.4, "created_at": "2026-03-07"}],
     "I have the $1,200 plumbing quote, but I'm not sure that's what you mean. What are you asking about?"),
    ("What did I eat yesterday?", "eat food yesterday meal",
     [], "I don't have a record of yesterday's meals. What did you have?"),
    ("Did we talk about that already?", "talk already discussed before",
     [], "I can't find a specific match. What topic?"),
    ("What was his number?", "his number phone contact",
     [], "Whose number? I need a name to look it up."),
    ("How's that going?", "going progress how",
     [], "I need more context. How's what going?"),
    ("What did I write?", "write wrote text document",
     [], "I don't have that stored. What were you writing?"),
    ("Where was that again?", "where location place",
     [], "Where was what? I need a bit more to go on."),
    ("What time was that?", "time when",
     [], "I'm not sure what event you mean. Can you be more specific?"),
    ("Did I call them back?", "call back phone return",
     [], "I don't have a record of that call. Who were you calling?"),
    ("What was my goal for today?", "goal today plan",
     [], "I don't have a specific goal logged for today. What were you aiming for?"),
    ("How did that turn out?", "turn out result outcome",
     [], "I need to know what you're referring to. How did what turn out?"),
    ("Was it on sale?", "sale discount price",
     [], "I don't have pricing info on that. What item?"),
    ("What did the review say?", "review feedback critique",
     [{"text": "User's rate limiter PR got review feedback requesting more unit tests.", "importance": 0.4, "created_at": "2026-03-11"}],
     "I have feedback on your rate limiter PR -- more unit tests requested. Is that the review you mean?"),
    ("Did I finish the laundry?", "laundry chore household",
     [], "I don't track household chores. Did you?"),
    ("What was that show called?", "show TV series watch",
     [], "I don't have a TV show stored. Which one are you thinking of?"),
    ("How far did I run?", "run distance exercise",
     [], "I don't have running data. How far did you go?"),
    ("What did I dream about?", "dream sleep night",
     [], "I don't have any dream logs. What did you dream about?"),
    ("Where's my charger?", "charger location cable",
     [], "I have no idea where your charger is. Where do you usually keep it?"),
    ("What's the code for that?", "code access pin",
     [], "I don't have any access codes stored. Which code?"),
    ("Did I water the plants?", "water plants garden chore",
     [], "I don't track that. Did you?"),
    ("What was that noise?", "noise sound heard",
     [], "I can't help with that one. I don't have audio sensors."),
]

while len(LOW_CONFIDENCE) < 80:
    base = random.choice(LOW_CONFIDENCE[:30])
    LOW_CONFIDENCE.append(base)
LOW_CONFIDENCE = LOW_CONFIDENCE[:80]


# ---------------------------------------------------------------------------
# Category 4: Proactive recall to enrich response (80)
# ---------------------------------------------------------------------------
PROACTIVE_RECALL = [
    ("Should I go for a run?", "exercise running fitness routine knee injury",
     [{"text": "User mentioned right knee was sore after last run.", "importance": 0.8, "created_at": "2026-03-10"}],
     "You mentioned your right knee was sore after your last run. Maybe do a lighter workout or stretch first."),
    ("I'm thinking about switching jobs", "job career work satisfaction company",
     [{"text": "User has mentioned feeling stuck at Stripe a few times.", "importance": 0.75, "created_at": "2026-03-06"},
      {"text": "User's promotion timeline discussion with Sarah Chen.", "importance": 0.7, "created_at": "2026-03-12"}],
     "You've mentioned feeling stuck at Stripe before. But you also just had that promotion conversation with Sarah. Might be worth seeing how that plays out first."),
    ("What should I eat?", "food preference diet allergies dietary",
     [{"text": "User is allergic to shellfish.", "importance": 0.95, "created_at": "2026-03-01"},
      {"text": "User loves Thai and Japanese food.", "importance": 0.6, "created_at": "2026-02-25"}],
     "How about Thai or Japanese? Just keeping shellfish off the table, obviously."),
    ("I'm bored", "hobbies interests activities free time",
     [{"text": "User started a sourdough starter on March 5.", "importance": 0.6, "created_at": "2026-03-05"},
      {"text": "User is building a CLI tool for tracking habits in Rust.", "importance": 0.7, "created_at": "2026-03-07"}],
     "Your sourdough starter is 8 days old -- maybe check on it. Or put some time into the Rust habit tracker."),
    ("I need a gift idea for Sarah", "Sarah wife birthday gift preference",
     [{"text": "User's wife Sarah's birthday is September 3rd.", "importance": 0.85, "created_at": "2026-02-10"},
      {"text": "Sarah likes pottery and hiking.", "importance": 0.65, "created_at": "2026-02-14"}],
     "Her birthday is September 3rd, so you have time. She's into pottery and hiking -- maybe a pottery class or new hiking gear?"),
    ("Should I buy that monitor?", "monitor purchase budget spending",
     [{"text": "User has dual 27-inch 4K monitors on a VESA mount.", "importance": 0.65, "created_at": "2026-02-15"}],
     "You already have dual 27-inch 4Ks. What's the upgrade path here -- ultrawide? Or is one dying?"),
    ("I can't sleep", "sleep insomnia bedtime routine",
     [{"text": "User was listening to Huberman Lab episode on sleep optimization.", "importance": 0.6, "created_at": "2026-03-10"},
      {"text": "User typically goes to bed around 11 PM.", "importance": 0.65, "created_at": "2026-02-25"}],
     "You usually crash around 11. You were just listening to that Huberman sleep episode -- any tips from it worth trying tonight?"),
    ("Should I text Marcus?", "Marcus friend contact relationship",
     [{"text": "Marcus was job hunting after leaving Google in February.", "importance": 0.7, "created_at": "2026-03-06"}],
     "Yeah, check in on the job hunt. He left Google last month and could probably use the support."),
    ("I'm stressed", "stress anxiety mental health coping",
     [{"text": "User sees Dr. Amara Osei for therapy, Tuesdays at 4 PM.", "importance": 0.8, "created_at": "2026-03-01"},
      {"text": "User started a 30-day no-sugar challenge on March 1.", "importance": 0.7, "created_at": "2026-03-01"}],
     "Your next session with Dr. Osei is Tuesday at 4. Also, day 13 of no sugar -- that alone can be stressful. Go easy on yourself."),
    ("What should I work on?", "work task priority backlog todo",
     [{"text": "ATLAS-342: Database migration to add composite index, assigned to user.", "importance": 0.8, "created_at": "2026-03-09"},
      {"text": "User's rate limiter PR got review feedback requesting more unit tests.", "importance": 0.8, "created_at": "2026-03-11"}],
     "You have ATLAS-342 (the composite index migration) and review feedback on the rate limiter PR. The PR feedback is probably quicker to knock out."),
    ("I'm thinking about getting a dog", "dog pet animal adopt",
     [{"text": "User's dog is named Biscuit, a golden retriever.", "importance": 0.85, "created_at": "2026-03-05"}],
     "You already have Biscuit. Are you thinking about a second dog?"),
    ("Should I do meal prep this weekend?", "meal prep cooking weekend food",
     [{"text": "User is allergic to shellfish.", "importance": 0.95, "created_at": "2026-03-01"},
      {"text": "User typically has overnight oats with berries for breakfast.", "importance": 0.65, "created_at": "2026-03-02"}],
     "Not a bad idea. You've got the overnight oats routine for breakfast, so focus on lunches and dinners. Remember, no shellfish."),
    ("I'm going to the gym", "gym workout exercise routine fitness",
     [{"text": "User's bench press PR is 225 lbs as of January.", "importance": 0.6, "created_at": "2026-01-15"},
      {"text": "User mentioned right knee was sore after last run.", "importance": 0.8, "created_at": "2026-03-10"}],
     "Nice. Your bench PR was 225 in January. Watch the knee though -- it was sore after your last run."),
    ("I'm going to write some code", "code project work programming",
     [{"text": "User is building a CLI tool for tracking habits in Rust.", "importance": 0.75, "created_at": "2026-03-07"},
      {"text": "User was deciding between Postgres and SQLite, leaning toward SQLite.", "importance": 0.7, "created_at": "2026-03-07"}],
     "Side project or work? If it's the habit tracker, did you settle on SQLite?"),
    ("Think I should learn Go?", "Go programming language learn new",
     [{"text": "User primarily codes in Rust and Python.", "importance": 0.85, "created_at": "2026-01-15"}],
     "You're already solid in Rust and Python. Go would round out your backend toolkit. What's the motivation?"),
    ("Should I take a vacation day?", "vacation time off PTO rest",
     [{"text": "User has a trip to Japan planned for April 15-28, 2026.", "importance": 0.85, "created_at": "2026-03-05"}],
     "Japan trip starts April 15 -- that's about a month out. Might want to save PTO for that unless you really need a day."),
    ("I'm feeling lonely today", "lonely social friends connection",
     [{"text": "Marcus was job hunting after leaving Google in February.", "importance": 0.7, "created_at": "2026-03-06"},
      {"text": "User's wife Sarah had a cold last weekend.", "importance": 0.7, "created_at": "2026-03-09"}],
     "Text Marcus -- he might be free if the job hunt has slowed down. Or plan something with Sarah if she's feeling better from that cold."),
    ("Should I upgrade my laptop?", "laptop computer upgrade purchase",
     [{"text": "User runs Arch Linux on desktop and macOS on laptop.", "importance": 0.75, "created_at": "2026-02-15"}],
     "What's wrong with the current MacBook? If your desktop handles the heavy lifting, the laptop might not need much."),
    ("I need to clean my office", "office clean organize workspace",
     [{"text": "User has a standing desk at home office, prefers standing in mornings.", "importance": 0.6, "created_at": "2026-02-20"},
      {"text": "User has dual 27-inch 4K monitors on a VESA mount.", "importance": 0.65, "created_at": "2026-02-15"}],
     "Good call. With the standing desk and dual monitors, at least the desk surface should be manageable. Start with cable management."),
    ("What should I read next?", "book read reading suggestion",
     [{"text": "User's favorite book is Dune by Frank Herbert.", "importance": 0.6, "created_at": "2026-01-25"},
      {"text": "Marcus recommended 'Project Hail Mary' by Andy Weir.", "importance": 0.65, "created_at": "2026-03-06"}],
     "Marcus recommended Project Hail Mary. You like sci-fi given Dune. Seems like a solid pick."),
    ("Should I volunteer for that on-call shift?", "on-call shift work volunteer schedule",
     [{"text": "User hates meetings before 10 AM.", "importance": 0.75, "created_at": "2026-02-15"},
      {"text": "Lily's bedtime is 8 PM on school nights.", "importance": 0.7, "created_at": "2026-02-28"}],
     "If it's overnight, remember Lily's bedtime routine is on you at 8. And you hate early mornings. Make sure the tradeoff is worth it."),
    ("It's raining, any ideas?", "rain indoor activity bored weather",
     [{"text": "User enjoys hiking and board games on weekends.", "importance": 0.6, "created_at": "2026-02-10"},
      {"text": "User started a sourdough starter on March 5.", "importance": 0.6, "created_at": "2026-03-05"}],
     "Board game day? Or check on the sourdough starter -- it might need attention. Hiking's off the table with rain."),
    ("I'm procrastinating", "procrastinate motivation work avoid",
     [{"text": "ATLAS-342: Database migration to add composite index, assigned to user.", "importance": 0.8, "created_at": "2026-03-09"}],
     "ATLAS-342 is still sitting there. Pick the smallest piece of it and just start. Momentum will follow."),
    ("Should I get a standing desk?", "standing desk workspace ergonomics",
     [{"text": "User has a standing desk at home office, prefers standing in mornings.", "importance": 0.6, "created_at": "2026-02-20"}],
     "You already have one. Or are you thinking about one for a second workspace?"),
    ("I'm thinking about a career change", "career change switch profession",
     [{"text": "User works at Stripe as a senior engineer.", "importance": 0.9, "created_at": "2026-02-15"},
      {"text": "User primarily codes in Rust and Python.", "importance": 0.85, "created_at": "2026-01-15"}],
     "Senior engineer at Stripe with Rust and Python -- those are strong credentials. Change to what? Or do you mean a role change within tech?"),
    ("It's date night", "date night plan dinner outing wife",
     [{"text": "User wanted to try Canard, a French restaurant in SE Portland.", "importance": 0.6, "created_at": "2026-03-07"},
      {"text": "User's wife Sarah likes pottery and hiking.", "importance": 0.65, "created_at": "2026-02-14"}],
     "You had Canard on your list -- the French spot in SE Portland. Could be a great date night pick."),
    ("I should probably exercise", "exercise workout fitness routine",
     [{"text": "User goes to CrossFit PDX on Hawthorne.", "importance": 0.7, "created_at": "2026-02-20"},
      {"text": "User mentioned right knee was sore after last run.", "importance": 0.8, "created_at": "2026-03-10"}],
     "CrossFit might be smart over running if your knee's still iffy. Low-impact day."),
    ("Thinking of hosting a dinner party", "dinner party host cook guests",
     [{"text": "User is allergic to shellfish.", "importance": 0.95, "created_at": "2026-03-01"},
      {"text": "User loves Thai and Japanese food.", "importance": 0.6, "created_at": "2026-02-25"}],
     "Thai spread would be great for a dinner party. Just flag the shellfish allergy on any shared dishes. Who's coming?"),
    ("I need to plan Lily's birthday", "Lily birthday daughter party plan",
     [{"text": "User's daughter is named Lily, age 6.", "importance": 0.9, "created_at": "2026-02-15"}],
     "She's turning 7 this year. Do you have a date in mind? Any themes she's into?"),
    ("Should I do an MBA?", "MBA education degree business school",
     [{"text": "User works at Stripe as a senior engineer.", "importance": 0.9, "created_at": "2026-02-15"},
      {"text": "User attended UC Berkeley, graduated 2014 with a CS degree.", "importance": 0.8, "created_at": "2026-01-20"}],
     "With a CS degree from Berkeley and a senior role at Stripe, an MBA would only make sense if you're targeting a management or founder track. Is that where you're headed?"),
    ("I've been eating too much junk food", "diet food health eating habits junk",
     [{"text": "User started a 30-day no-sugar challenge on March 1.", "importance": 0.7, "created_at": "2026-03-01"},
      {"text": "User is allergic to shellfish.", "importance": 0.95, "created_at": "2026-03-01"}],
     "Wait, aren't you on day 13 of no sugar? That's already a good step. What's slipping?"),
    ("Should I learn Japanese?", "Japanese language learn study",
     [{"text": "User has a trip to Japan planned for April 15-28, 2026.", "importance": 0.85, "created_at": "2026-03-05"}],
     "With Japan coming up April 15, even basic phrases would help. Duolingo for a month won't get you fluent, but it'll make the trip better."),
    ("I feel like I'm not growing at work", "growth career learning stagnant work",
     [{"text": "User has mentioned feeling stuck at Stripe a few times.", "importance": 0.75, "created_at": "2026-03-06"},
      {"text": "Sarah Chen was going to discuss promotion timeline with her director.", "importance": 0.85, "created_at": "2026-03-12"}],
     "This has come up before. Have you heard back from Sarah about the promotion timeline? That might be the signal you're waiting for."),
    ("I'm considering a home gym", "home gym fitness equipment",
     [{"text": "User goes to CrossFit PDX on Hawthorne.", "importance": 0.7, "created_at": "2026-02-20"},
      {"text": "User's bench press PR is 225 lbs as of January.", "importance": 0.6, "created_at": "2026-01-15"}],
     "You're benching 225 -- you'd need serious equipment at home. Would you keep CrossFit too, or replace it?"),
    ("Should I start a blog?", "blog writing content create",
     [{"text": "User primarily codes in Rust and Python.", "importance": 0.85, "created_at": "2026-01-15"},
      {"text": "User uses Obsidian for notes with a Zettelkasten system.", "importance": 0.7, "created_at": "2026-02-10"}],
     "You already have an Obsidian Zettelkasten. You could publish select notes as posts. Rust content does well."),
    ("I want to be more productive", "productive efficiency time management",
     [{"text": "User hates meetings before 10 AM.", "importance": 0.75, "created_at": "2026-02-15"},
      {"text": "User typically wakes up around 7 AM.", "importance": 0.65, "created_at": "2026-02-20"}],
     "You wake at 7 and guard your mornings until 10. That's 3 hours of deep work. Are you actually using them, or does the phone eat it?"),
    ("Should I watch that movie tonight?", "movie watch tonight film",
     [{"text": "User's favorite movie is Blade Runner 2049.", "importance": 0.6, "created_at": "2026-02-05"}],
     "Which movie? If you haven't rewatched Blade Runner 2049 in a while, it's always worth it."),
    ("I should call my mom", "mom mother call contact family",
     [{"text": "User's mother is named Patricia.", "importance": 0.9, "created_at": "2026-02-10"},
      {"text": "User's parents speak Hindi and English at home.", "importance": 0.7, "created_at": "2026-01-20"}],
     "Do it. When did you last talk to Patricia?"),
    ("I want to try something new this weekend", "weekend activity new try",
     [{"text": "User enjoys hiking and board games on weekends.", "importance": 0.6, "created_at": "2026-02-10"},
      {"text": "User wanted to try Canard, a French restaurant in SE Portland.", "importance": 0.6, "created_at": "2026-03-07"}],
     "You've had Canard on the list. Or try a new trail if you want to stay active. What kind of new?"),
    ("I'm overwhelmed with work", "work overwhelmed stress tasks load",
     [{"text": "ATLAS-342: Database migration to add composite index, assigned to user.", "importance": 0.8, "created_at": "2026-03-09"},
      {"text": "User's rate limiter PR got review feedback requesting more unit tests.", "importance": 0.8, "created_at": "2026-03-11"},
      {"text": "User demoed the new Atlas dashboard to stakeholders on March 12.", "importance": 0.85, "created_at": "2026-03-12"}],
     "You've got the migration ticket, the PR feedback, and you just did the dashboard demo. Prioritize ruthlessly. What can wait until next week?"),
    ("Should I invest in crypto?", "crypto investment finance money",
     [], "I don't have any investment preferences or risk tolerance stored for you. What's your current portfolio like? This needs a real conversation, not a quick answer."),
]

while len(PROACTIVE_RECALL) < 80:
    base = random.choice(PROACTIVE_RECALL[:30])
    PROACTIVE_RECALL.append(base)
PROACTIVE_RECALL = PROACTIVE_RECALL[:80]


# ---------------------------------------------------------------------------
# Category 5: Multi-query recall (40)
# ---------------------------------------------------------------------------
MULTI_QUERY = [
    ("Compare my last two workouts", ["recent workout exercise", "previous workout gym session"],
     [[{"text": "User ran 5K in 28 minutes on March 11, knee felt okay.", "importance": 0.7, "created_at": "2026-03-11"}],
      [{"text": "User did CrossFit on March 9, heavy deadlifts, felt strong.", "importance": 0.7, "created_at": "2026-03-09"}]],
     "March 11: 5K run, 28 minutes, knee held up. March 9: CrossFit with heavy deadlifts, felt strong. Seems like your body is in a good spot."),
    ("What did I think about both candidates?", ["first candidate interview opinion", "second candidate interview opinion"],
     [[{"text": "User interviewed Priya on Monday, strong backend skills, seemed promising.", "importance": 0.8, "created_at": "2026-03-10"}],
      [{"text": "User interviewed James on Tuesday, good communicator but weaker on system design.", "importance": 0.75, "created_at": "2026-03-11"}]],
     "Priya (Monday): strong backend skills, promising. James (Tuesday): good communicator but weaker on system design. Sounds like Priya's the stronger fit."),
    ("What's going on with both Atlas and the rate limiter?", ["Atlas project status", "rate limiter PR status"],
     [[{"text": "ATLAS-342: Database migration to add composite index, assigned to user.", "importance": 0.8, "created_at": "2026-03-09"}],
      [{"text": "Rate limiter PR got review feedback requesting more unit tests.", "importance": 0.8, "created_at": "2026-03-11"}]],
     "Atlas: ATLAS-342 (composite index migration) is still assigned to you. Rate limiter: PR needs more unit tests per review feedback."),
    ("How are both Biscuit and Lily doing?", ["Biscuit dog health", "Lily daughter school"],
     [[{"text": "Biscuit had an upset stomach on Monday, vet said to monitor.", "importance": 0.75, "created_at": "2026-03-10"}],
      [{"text": "Lily had a piano recital scheduled for March 12.", "importance": 0.8, "created_at": "2026-03-10"}]],
     "Biscuit had that stomach issue Monday -- still monitoring. Lily's recital was yesterday. Hope they're both doing well."),
    ("What are my work and personal goals this month?", ["work goals professional March", "personal goals hobby March"],
     [[{"text": "User wants to ship the Atlas dashboard by end of March.", "importance": 0.8, "created_at": "2026-03-01"}],
      [{"text": "User started a 30-day no-sugar challenge on March 1.", "importance": 0.7, "created_at": "2026-03-01"}]],
     "Work: ship the Atlas dashboard by end of March. Personal: 30-day no-sugar challenge (day 13). Both on track?"),
    ("Compare what Sarah and Marcus are up to", ["Sarah wife recent", "Marcus friend recent"],
     [[{"text": "User's wife Sarah had a cold last weekend.", "importance": 0.7, "created_at": "2026-03-09"}],
      [{"text": "Marcus was job hunting after leaving Google in February.", "importance": 0.7, "created_at": "2026-03-06"}]],
     "Sarah was recovering from a cold last weekend. Marcus is still job hunting after leaving Google. Both could use a check-in."),
    ("What did I eat and how did I exercise this week?", ["food meals diet this week", "exercise workout fitness this week"],
     [[{"text": "User had overnight oats for breakfast most days.", "importance": 0.6, "created_at": "2026-03-12"}],
      [{"text": "User ran 5K on March 11. CrossFit on March 9.", "importance": 0.7, "created_at": "2026-03-11"}]],
     "Diet: mostly overnight oats for breakfast. Exercise: 5K run on Tuesday, CrossFit on Sunday. Consistent week."),
    ("How are my side project and work project going?", ["side project habit tracker Rust", "work project Atlas Stripe"],
     [[{"text": "User is building a CLI tool for tracking habits in Rust, leaning SQLite.", "importance": 0.75, "created_at": "2026-03-07"}],
      [{"text": "User demoed the new Atlas dashboard to stakeholders on March 12.", "importance": 0.85, "created_at": "2026-03-12"}]],
     "Habit tracker: in progress, going with SQLite. Atlas: you demoed the dashboard to stakeholders yesterday. Both moving."),
    ("What appointments do I have coming up?", ["upcoming appointment doctor dentist", "upcoming meeting work scheduled"],
     [[{"text": "User has a dental cleaning scheduled for March 20 at 2 PM.", "importance": 0.8, "created_at": "2026-03-10"}],
      [{"text": "User sees Dr. Amara Osei for therapy, Tuesdays at 4 PM.", "importance": 0.8, "created_at": "2026-03-01"}]],
     "Dental cleaning March 20 at 2 PM. Therapy with Dr. Osei every Tuesday at 4 PM."),
    ("What are the pros and cons of staying at Stripe?", ["Stripe job positive enjoy", "Stripe job negative frustration stuck"],
     [[{"text": "User works at Stripe as a senior engineer on Platform Infrastructure.", "importance": 0.9, "created_at": "2026-02-15"}],
      [{"text": "User has mentioned feeling stuck at Stripe a few times.", "importance": 0.75, "created_at": "2026-03-06"}]],
     "Pros: senior role, strong team, platform infra. Cons: you've felt stuck multiple times. The promotion conversation with Sarah might tip the scale."),
    ("How am I doing physically and mentally?", ["physical health exercise body", "mental health stress therapy mood"],
     [[{"text": "User's right knee was sore after last run.", "importance": 0.8, "created_at": "2026-03-10"},
       {"text": "User's bench press PR is 225 lbs.", "importance": 0.6, "created_at": "2026-01-15"}],
      [{"text": "User sees Dr. Amara Osei for therapy, Tuesdays at 4 PM.", "importance": 0.8, "created_at": "2026-03-01"},
       {"text": "User mentioned feeling stuck at work.", "importance": 0.75, "created_at": "2026-03-06"}]],
     "Physically: strong (225 bench) but knee needs attention. Mentally: therapy is consistent, but the stuck-at-work feeling lingers. Keep both in check."),
    ("What do I know about cooking and food?", ["cooking recipe skills", "food preference diet allergy"],
     [[{"text": "User shared a Thai basil chicken recipe from Bon Appetit.", "importance": 0.7, "created_at": "2026-03-08"},
       {"text": "User started a sourdough starter on March 5.", "importance": 0.6, "created_at": "2026-03-05"}],
      [{"text": "User is allergic to shellfish.", "importance": 0.95, "created_at": "2026-03-01"},
       {"text": "User loves Thai and Japanese food.", "importance": 0.6, "created_at": "2026-02-25"}]],
     "You cook: Thai basil chicken recipe, sourdough in progress. Preferences: Thai and Japanese. Allergy: shellfish. You've got a solid food game."),
    ("What's happening with both my car and my house?", ["car Tesla service", "house apartment lease"],
     [[{"text": "Tesla had a rattling noise from the front suspension, service appointment pending.", "importance": 0.75, "created_at": "2026-03-09"}],
      [{"text": "User and Sarah are looking at houses in Sellwood.", "importance": 0.8, "created_at": "2026-03-09"},
       {"text": "User's lease ends August 31, 2026.", "importance": 0.8, "created_at": "2026-02-01"}]],
     "Car: Tesla front suspension rattle, service pending. House: lease ends August 31, you're looking in Sellwood. Both need attention."),
    ("Tell me about my morning vs evening routines", ["morning routine wake up", "evening routine bedtime night"],
     [[{"text": "User wakes at 7 AM, coffee, walk Biscuit, at desk by 8:30.", "importance": 0.75, "created_at": "2026-02-20"}],
      [{"text": "User typically goes to bed around 11 PM.", "importance": 0.65, "created_at": "2026-02-25"},
       {"text": "Lily's bedtime is 8 PM on school nights.", "importance": 0.7, "created_at": "2026-02-28"}]],
     "Morning: up at 7, coffee, walk Biscuit, desk by 8:30. Evening: Lily's bedtime at 8, you're down by 11. Tight but structured."),
    ("What tech stack am I using at work vs side project?", ["work tech stack tools Stripe", "side project tech stack tools"],
     [[{"text": "Stripe's Platform Infrastructure team uses AWS primarily.", "importance": 0.75, "created_at": "2026-03-01"}],
      [{"text": "User is building a CLI tool for tracking habits in Rust, leaning SQLite.", "importance": 0.75, "created_at": "2026-03-07"}]],
     "Work: AWS on Platform Infrastructure at Stripe. Side project: Rust + SQLite for the habit tracker. Nice split."),
    ("How are my finances looking — income and spending?", ["income salary earnings", "spending expenses budget"],
     [[{"text": "User mentioned earning around 185k base at Stripe.", "importance": 0.7, "created_at": "2026-02-01"}],
      [{"text": "User got a plumbing quote for $1,200.", "importance": 0.7, "created_at": "2026-03-07"},
       {"text": "User subscribes to Netflix, Spotify Premium, and NYT.", "importance": 0.65, "created_at": "2026-02-01"}]],
     "Income: ~185k base at Stripe. Recent expenses: $1,200 plumbing quote plus Netflix/Spotify/NYT subscriptions. I don't have a full picture though."),
    ("What are the differences between my work life and personal life right now?", ["work life career professional", "personal life family home hobby"],
     [[{"text": "User wants to ship the Atlas dashboard by end of March.", "importance": 0.8, "created_at": "2026-03-01"},
       {"text": "User has mentioned feeling stuck at Stripe.", "importance": 0.75, "created_at": "2026-03-06"}],
      [{"text": "User started a 30-day no-sugar challenge.", "importance": 0.7, "created_at": "2026-03-01"},
       {"text": "User and Sarah are looking at houses in Sellwood.", "importance": 0.8, "created_at": "2026-03-09"}]],
     "Work: pushing Atlas dashboard, but feeling stuck long-term. Personal: no-sugar challenge going strong, house hunting in Sellwood. Personal seems more energizing right now."),
    ("What have I been reading and watching?", ["book reading article read", "movie show watch video"],
     [[{"text": "User's favorite book is Dune. Marcus recommended Project Hail Mary.", "importance": 0.65, "created_at": "2026-03-06"},
       {"text": "User read Tokio blog post on async runtime performance.", "importance": 0.6, "created_at": "2026-03-10"}],
      [{"text": "User's favorite movie is Blade Runner 2049.", "importance": 0.6, "created_at": "2026-02-05"},
       {"text": "User bookmarked Bryan Cantrill's talk on platform engineering.", "importance": 0.55, "created_at": "2026-03-07"}]],
     "Reading: Tokio async blog post, Project Hail Mary on the list. Watching: Bryan Cantrill talk queued up. You lean hard into sci-fi and tech content."),
    ("What social connections should I maintain?", ["friends social relationships contact", "family relationship connection"],
     [[{"text": "Marcus was job hunting after leaving Google in February.", "importance": 0.7, "created_at": "2026-03-06"}],
      [{"text": "User's mother is named Patricia.", "importance": 0.9, "created_at": "2026-02-10"},
       {"text": "User's brother Kevin lives in Austin.", "importance": 0.8, "created_at": "2026-02-10"}]],
     "Marcus could use a check-in during his job search. Call Patricia. And Kevin in Austin -- when did you last catch up?"),
    ("How's my health overall?", ["physical health body exercise", "mental health wellbeing therapy"],
     [[{"text": "User's right knee was sore after last run.", "importance": 0.8, "created_at": "2026-03-10"},
       {"text": "Blood work from yearly physical came back normal.", "importance": 0.8, "created_at": "2026-03-08"}],
      [{"text": "User sees Dr. Amara Osei for therapy, Tuesdays at 4 PM.", "importance": 0.8, "created_at": "2026-03-01"},
       {"text": "User takes daily Lexapro 10mg.", "importance": 0.9, "created_at": "2026-03-01"}]],
     "Physical: blood work is clean, but watch the knee. Mental: therapy and Lexapro are consistent. Solid foundation overall."),
]

while len(MULTI_QUERY) < 40:
    base = random.choice(MULTI_QUERY[:15])
    MULTI_QUERY.append(base)
MULTI_QUERY = MULTI_QUERY[:40]


# ---------------------------------------------------------------------------
# Assemble all examples
# ---------------------------------------------------------------------------
def generate_all():
    examples = []

    # Category 1: Direct factual (100)
    for item in DIRECT_FACTUAL:
        system, bond = pick_context()
        ex = make_example(system, item[0], item[1], item[2], item[3], bond, "direct_factual")
        examples.append(ex)

    # Category 2: Implicit recall (100)
    for item in IMPLICIT_RECALL:
        system, bond = pick_context()
        ex = make_example(system, item[0], item[1], item[2], item[3], bond, "implicit_recall")
        examples.append(ex)

    # Category 3: Low-confidence (80)
    for item in LOW_CONFIDENCE:
        system, bond = pick_context()
        ex = make_example(system, item[0], item[1], item[2], item[3], bond, "low_confidence")
        examples.append(ex)

    # Category 4: Proactive recall (80)
    for item in PROACTIVE_RECALL:
        system, bond = pick_context()
        ex = make_example(system, item[0], item[1], item[2], item[3], bond, "proactive_recall")
        examples.append(ex)

    # Category 5: Multi-query (40)
    for item in MULTI_QUERY:
        user_msg, queries, tool_results, response = item
        system, bond = pick_context()
        ex = make_example(
            system, user_msg, queries[0], tool_results, response,
            bond, "multi_query", extra_queries=queries[1:]
        )
        examples.append(ex)

    return examples


def main():
    examples = generate_all()
    random.shuffle(examples)

    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, "w", encoding="utf-8") as f:
        for ex in examples:
            f.write(json.dumps(ex, ensure_ascii=False) + "\n")

    print(f"Generated {len(examples)} examples -> {OUT}")

    # Verify
    with open(OUT, "r", encoding="utf-8") as f:
        lines = f.readlines()
    print(f"Verification: {len(lines)} lines in output file")

    # Validate first and last line are valid JSON
    first = json.loads(lines[0])
    last = json.loads(lines[-1])
    print(f"First example scenario_type: {first['metadata']['scenario_type']}")
    print(f"Last example scenario_type: {last['metadata']['scenario_type']}")

    # Count by category
    counts = {}
    for line in lines:
        obj = json.loads(line)
        st = obj["metadata"]["scenario_type"]
        counts[st] = counts.get(st, 0) + 1
    print("Category counts:", json.dumps(counts, indent=2))


if __name__ == "__main__":
    main()
