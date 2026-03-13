#!/usr/bin/env python3
"""Generate 200 JSONL training examples for memory conflict detection, contradiction surfacing,
user-driven resolution, and entity anomaly handling."""

import json
import random
import uuid
import os

random.seed(91)

NAMES = ["Alex", "Sam", "Maya", "Jordan", "Priya", "Leo", "Kai", "Noor", "Tess", "Ravi",
         "Zoe", "Finn", "Anya", "Cole", "Devi"]

BOND_STAGES = ["stranger", "acquaintance", "trusted", "deep", "partner_in_crime"]

BOND_INSTRUCTIONS = {
    "stranger": (
        "Bond stage: STRANGER. Be helpful and polite but reserved. "
        "Use full sentences, no nicknames. Be concise. No filler phrases. No emoji."
    ),
    "acquaintance": (
        "Bond stage: ACQUAINTANCE. You're friendly and warm but still respectful. "
        "You remember some past interactions. Be concise. No filler phrases. No emoji."
    ),
    "trusted": (
        "Bond stage: TRUSTED. You're casual and direct. You share opinions freely. "
        "You reference shared history naturally. Be concise. No filler phrases. No emoji."
    ),
    "deep": (
        "Bond stage: DEEP. You know the user well. You're warm, direct, sometimes playful. "
        "You anticipate needs. Be concise. No filler phrases. No emoji."
    ),
    "partner_in_crime": (
        "Bond stage: PARTNER_IN_CRIME. You're the user's closest digital companion. "
        "Very casual, witty, direct. You finish each other's sentences. No filler. No emoji."
    ),
}

TIMES = [
    "2026-03-13 08:15 AM", "2026-03-13 09:30 AM", "2026-03-13 11:45 AM",
    "2026-03-13 01:20 PM", "2026-03-13 03:00 PM", "2026-03-13 05:30 PM",
    "2026-03-13 07:15 PM", "2026-03-13 09:45 PM", "2026-03-12 10:00 AM",
    "2026-03-11 02:30 PM", "2026-03-10 08:00 AM", "2026-03-09 06:00 PM",
]


def call_id():
    return f"call_{uuid.uuid4().hex[:12]}"


def mem_id():
    return f"mem_{uuid.uuid4().hex[:8]}"


def make_tool_call(tool_name, arguments):
    return {
        "id": call_id(),
        "type": "function",
        "function": {
            "name": tool_name,
            "arguments": json.dumps(arguments),
        },
    }


def make_system_prompt(name, bond_stage, time, memories=None, open_conflicts=None):
    base = (
        f"You are {name}, the user's personal companion. "
        f"{BOND_INSTRUCTIONS[bond_stage]} "
        f"You have persistent memory. When you notice contradictions in your memories, "
        f"surface them naturally and ask the user to clarify. Don't assume — ask. "
        f"Store the correction when resolved.\n\n"
        f"Current time: {time}.\n"
    )
    if memories:
        base += "\nRelevant memories:\n"
        for m in memories:
            base += f"- [{m['rid']}] {m['text']} (domain: {m['domain']}, stored: {m['date']})\n"
    if open_conflicts:
        base += "\nOpen memory conflicts:\n"
        for c in open_conflicts:
            base += (f"- [{c['type']}] \"{c['mem_a']}\" vs \"{c['mem_b']}\" "
                     f"(entity: {c['entity']}, conflict_id: {c['id']}, detected: {c['date']})\n")
    base += "\nTools available: recall, remember, save_user_fact, resolve_conflicts, forget_memory, update_memory, relate_entities"
    return base


# ---------------------------------------------------------------------------
# CATEGORY 1: New info contradicts stored memory (60 examples)
# ---------------------------------------------------------------------------
# (user_msg, recall_query, old_memory_text, old_domain, old_date,
#  companion_question, user_answer, new_fact, new_domain, forget_reason)
CAT1_SCENARIOS = [
    # Work/Job changes
    ("Starting my new role at Stripe next week, pretty excited.",
     "work job company employer",
     "User works at Google as a software engineer", "work", "2025-11-02",
     "Congrats on the Stripe move. I had you down as being at Google — should I update that?",
     "Yeah, I'm leaving Google for Stripe.",
     "User works at Stripe, left Google in March 2026", "work",
     "Outdated — user moved from Google to Stripe"),
    ("I got promoted to senior engineer yesterday!",
     "job title role position",
     "User is a mid-level engineer at Datadog", "work", "2025-09-15",
     "That's great news. I had your title as mid-level engineer — you're senior now?",
     "Yep, senior engineer as of this week.",
     "User is a senior engineer at Datadog, promoted March 2026", "work",
     "Outdated — user promoted to senior engineer"),
    ("Working from the new Meta office today, the campus is massive.",
     "work company employer job",
     "User works at Amazon in Seattle", "work", "2025-08-20",
     "Sounds impressive. I have you listed as working at Amazon — did you switch to Meta?",
     "Yeah I joined Meta last month.",
     "User works at Meta, left Amazon", "work",
     "Outdated — user left Amazon for Meta"),
    ("My new manager Sarah is really great, so much better than the last one.",
     "manager boss work team",
     "User's manager is David at the engineering team", "work", "2025-10-10",
     "Glad to hear it. I had David as your manager — is Sarah the replacement?",
     "David left the company, Sarah took over in February.",
     "User's manager is Sarah, replaced David who left", "work",
     "Outdated — David no longer user's manager"),
    ("The commute from our new house is only 10 minutes now.",
     "commute work travel home",
     "User's commute is 45 minutes by train", "routine", "2025-07-15",
     "Nice upgrade from the 45-minute train ride I had noted. Did you move closer to work?",
     "We bought a house near the office last month.",
     "User's commute is 10 minutes, moved closer to work", "routine",
     "Outdated — user moved, commute changed from 45 min to 10 min"),
    ("Just handed in my resignation at Apple today.",
     "work job company employer Apple",
     "User works at Apple in the iOS team", "work", "2025-12-01",
     "Big step. What's the plan after Apple?",
     "Taking a month off then joining a startup.",
     "User resigned from Apple, planning to join a startup after a break", "work",
     "User resigned from Apple in March 2026"),
    ("I switched to the data science team last week.",
     "team role work department",
     "User is on the backend engineering team", "work", "2025-10-20",
     "Oh interesting pivot. So you're doing data science now instead of backend?",
     "Yeah, internal transfer. Same company, different team.",
     "User is on the data science team, transferred from backend engineering", "work",
     "Outdated — user transferred from backend to data science team"),
    ("Freelancing full time now, no more corporate life.",
     "work job employment company",
     "User works at Microsoft in the Azure team", "work", "2025-06-10",
     "That's a big change from Microsoft. When did you make the switch?",
     "Left last month. Freelancing suits me better.",
     "User is a full-time freelancer, left Microsoft", "work",
     "Outdated — user left Microsoft to freelance"),
    # Location changes
    ("Just got settled into the new apartment in Austin.",
     "location home city address",
     "User lives in Portland, Oregon", "location", "2025-09-01",
     "Austin, nice. I had Portland as your home base — all moved?",
     "Yep, fully moved to Austin two weeks ago.",
     "User lives in Austin, Texas, moved from Portland in March 2026", "location",
     "Outdated — user moved from Portland to Austin"),
    ("London is amazing, so glad we decided to relocate.",
     "location home city country",
     "User lives in San Francisco", "location", "2025-11-15",
     "London's a great city. So you've moved from San Francisco permanently?",
     "Yeah, my partner got a job here so we moved.",
     "User lives in London, relocated from San Francisco", "location",
     "Outdated — user relocated from SF to London"),
    ("Unpacking boxes in the new Denver place.",
     "home location city move",
     "User lives in Chicago", "location", "2025-10-01",
     "Denver, exciting. I still had Chicago on file — should I update?",
     "Yeah, Chicago's done. Denver now.",
     "User lives in Denver, moved from Chicago", "location",
     "Outdated — user moved from Chicago to Denver"),
    ("Finally back in my hometown, moved back to Seattle.",
     "location home city",
     "User lives in New York City", "location", "2025-08-12",
     "Welcome home. I had NYC as your location — back in Seattle for good?",
     "For good this time. NYC was fun but Seattle is home.",
     "User lives in Seattle, moved back from NYC", "location",
     "Outdated — user moved back to Seattle from NYC"),
    # Diet/Food preference changes
    ("Had the best steak last night at this new place downtown.",
     "diet food preferences vegetarian",
     "User is vegetarian", "preferences", "2025-07-20",
     "Wait, I thought you were vegetarian? Did something change?",
     "I stopped being vegetarian about three months ago. Missed meat too much.",
     "User is no longer vegetarian, started eating meat again", "preferences",
     "Outdated — user is no longer vegetarian"),
    ("I've been doing keto for the past month and feeling great.",
     "diet food eating nutrition",
     "User follows a vegan diet", "preferences", "2025-11-05",
     "That's a shift from the vegan diet I had noted. Fully switched to keto?",
     "Yeah, vegan wasn't working for me. Keto is much better.",
     "User follows a keto diet, switched from vegan", "preferences",
     "Outdated — user switched from vegan to keto"),
    ("Started eating dairy again, my doctor recommended it for the calcium.",
     "diet food dairy lactose",
     "User is lactose intolerant, avoids dairy", "health", "2025-09-22",
     "Oh interesting. I had you as lactose intolerant — did that change?",
     "Turns out I'm not actually intolerant, just sensitivity. Supplements help.",
     "User eating dairy again, not fully lactose intolerant", "health",
     "Outdated — user can eat dairy with supplements"),
    # Relationship changes
    ("My wife and I are celebrating our first anniversary next month.",
     "relationship partner marriage",
     "User is single", "relationships", "2025-06-01",
     "Congratulations! I had you down as single — when did you get married?",
     "Got married last April, guess I never mentioned it.",
     "User is married, anniversary in April", "relationships",
     "Outdated — user is now married"),
    ("Going through the divorce process now, it's been rough.",
     "relationship marriage partner spouse",
     "User is married to Lisa", "relationships", "2025-10-15",
     "I'm sorry to hear that. That must be really difficult. Is there anything I can help with?",
     "Just need to update your records I guess. Lisa and I split up.",
     "User is divorcing Lisa", "relationships",
     "Outdated — user and Lisa are divorcing"),
    ("My girlfriend Emma and I just adopted a dog!",
     "relationship partner girlfriend",
     "User is dating someone named Rachel", "relationships", "2025-08-05",
     "A dog, that's exciting. I had Rachel as your girlfriend though — are you with Emma now?",
     "Rachel and I broke up months ago. Emma and I have been together since November.",
     "User is dating Emma, previously dated Rachel", "relationships",
     "Outdated — user broke up with Rachel, now dating Emma"),
    # Tech/Tool preferences
    ("Loving Neovim, can't believe I used VS Code for so long.",
     "editor IDE tools development",
     "User prefers VS Code for development", "preferences", "2025-09-10",
     "Neovim convert! Should I update your editor preference from VS Code?",
     "Definitely. Neovim all the way now.",
     "User prefers Neovim for development, switched from VS Code", "preferences",
     "Outdated — user switched from VS Code to Neovim"),
    ("Switched our whole stack to TypeScript last quarter.",
     "programming language stack tech",
     "User's team uses Python for backend", "tech", "2025-07-25",
     "Full TypeScript stack — so you moved off Python for the backend too?",
     "Yeah, everything is TypeScript now. Backend, frontend, all of it.",
     "User's team uses TypeScript for full stack, moved off Python", "tech",
     "Outdated — team migrated from Python to TypeScript"),
    ("Finally ditched Chrome, Firefox is so much better for privacy.",
     "browser chrome firefox tools",
     "User uses Chrome as primary browser", "preferences", "2025-11-20",
     "Firefox fan now. I had Chrome listed as your browser — want me to update?",
     "Please do. Chrome is gone from all my devices.",
     "User uses Firefox as primary browser, switched from Chrome", "preferences",
     "Outdated — user switched from Chrome to Firefox"),
    # Hobby/Interest changes
    ("Been really into rock climbing lately, go three times a week.",
     "hobbies interests exercise activities",
     "User's main hobby is running marathons", "hobbies", "2025-08-30",
     "Rock climbing, nice. Still doing the marathon running too or fully switched?",
     "Gave up running after my knee injury. Climbing is easier on the joints.",
     "User's main hobby is rock climbing, stopped running due to knee injury", "hobbies",
     "Outdated — user stopped running, now does rock climbing"),
    ("Learning Japanese now, dropped the Spanish classes.",
     "language learning study education",
     "User is learning Spanish", "education", "2025-10-08",
     "Japanese, cool. So Spanish is on hold?",
     "Dropped it entirely. Japanese is way more interesting to me.",
     "User is learning Japanese, dropped Spanish", "education",
     "Outdated — user dropped Spanish, learning Japanese"),
    ("Sold all my crypto, I'm done with that world.",
     "investments crypto finance",
     "User invests in cryptocurrency, holds Bitcoin and Ethereum", "finance", "2025-09-14",
     "Really? You were pretty into the crypto space. All of it gone?",
     "Every last coin. Index funds only now.",
     "User sold all crypto, invests in index funds only", "finance",
     "Outdated — user sold all cryptocurrency"),
    # Pet changes
    ("Our new cat Mochi is settling in nicely.",
     "pets animals cat dog",
     "User has a dog named Biscuit, no cats", "personal", "2025-07-01",
     "Mochi, cute name. Do you still have Biscuit too?",
     "Yeah, Biscuit and Mochi are getting along great.",
     "User has a dog named Biscuit and a cat named Mochi", "personal",
     "Incomplete — user also has a cat now"),
    # Schedule changes
    ("I'm working nights now, the shift change happened last week.",
     "schedule work hours routine",
     "User works 9-5 daytime schedule", "routine", "2025-11-01",
     "Night shift, that's a big change. Should I adjust when I check in with you?",
     "Yeah, I sleep until about 2 PM now.",
     "User works night shift, sleeps until ~2 PM", "routine",
     "Outdated — user switched from day shift to night shift"),
    ("Four day work week starting this month, Fridays off!",
     "schedule work days routine",
     "User works Monday through Friday", "routine", "2025-10-05",
     "Nice perk. So Monday through Thursday now?",
     "Exactly, every Friday off going forward.",
     "User works Monday through Thursday, four day work week", "routine",
     "Outdated — user switched to 4-day work week"),
    # Education
    ("Just enrolled in an MBA program at Wharton.",
     "education school degree study",
     "User has a CS degree, no graduate education", "education", "2025-06-15",
     "Wharton MBA, impressive. When does it start?",
     "Classes begin in September, part-time while I work.",
     "User enrolled in part-time MBA at Wharton, starts September 2026", "education",
     "Incomplete — user pursuing MBA in addition to CS degree"),
    ("Dropped out of the bootcamp, wasn't worth the money.",
     "education bootcamp course learning",
     "User is attending a data science bootcamp", "education", "2025-12-10",
     "Sorry to hear it wasn't a fit. What turned you off?",
     "Teaching quality was awful. Going self-taught instead.",
     "User dropped out of data science bootcamp, self-studying instead", "education",
     "Outdated — user left the bootcamp"),
    # Health
    ("Doctor cleared me to exercise again, the injury is healed.",
     "health injury exercise medical",
     "User has a back injury, restricted from exercise", "health", "2025-11-28",
     "Great news about the recovery. Back to full activity?",
     "Yep, back is fully healed. Starting gym again tomorrow.",
     "User recovered from back injury, cleared for exercise", "health",
     "Outdated — back injury is healed"),
]

# ---------------------------------------------------------------------------
# CATEGORY 2: Proactive conflict surfacing (40 examples)
# ---------------------------------------------------------------------------
# (conflict_type, mem_a, mem_b, entity, companion_intro, user_resolution, resolved_fact, domain)
CAT2_SCENARIOS = [
    ("IdentityFact", "User lives in Portland", "User lives in Austin",
     "location", "Quick thing — I have conflicting notes about where you live. Portland or Austin?",
     "Austin. I moved there last month.",
     "User lives in Austin, moved from Portland", "location"),
    ("IdentityFact", "User's name is Mike", "User's preferred name is Michael",
     "name", "I've got you stored as both Mike and Michael. Which do you prefer?",
     "Mike is fine, that's what everyone calls me.",
     "User's preferred name is Mike", "identity"),
    ("IdentityFact", "User's birthday is March 15", "User's birthday is March 17",
     "birthday", "I have two different birthdays for you — March 15 and March 17. Which is right?",
     "March 15th, I must have mistyped the other time.",
     "User's birthday is March 15", "identity"),
    ("IdentityFact", "User works at Netflix", "User works at Spotify",
     "employer", "I have both Netflix and Spotify as your employer. Which one's current?",
     "Spotify. I left Netflix in January.",
     "User works at Spotify, left Netflix in January 2026", "work"),
    ("IdentityFact", "User is 28 years old", "User is 31 years old",
     "age", "Got a conflict in my notes — your age shows as both 28 and 31. Which is accurate?",
     "I'm 31. The 28 was probably from a while back.",
     "User is 31 years old", "identity"),
    ("Preference", "User prefers dark mode", "User prefers light mode",
     "display_preference", "I've got both dark mode and light mode listed as your preference. Which is it these days?",
     "Dark mode, always dark mode.",
     "User prefers dark mode", "preferences"),
    ("Preference", "User's favorite cuisine is Italian", "User's favorite cuisine is Thai",
     "food_preference", "I have Italian and Thai both listed as your favorite cuisine. Do you have a preference?",
     "Thai for sure. Italian is good but Thai is number one.",
     "User's favorite cuisine is Thai, also likes Italian", "preferences"),
    ("Preference", "User prefers morning workouts", "User works out in the evening",
     "workout_time", "My notes say you work out in the morning and also in the evening. What's your actual schedule?",
     "Evenings now. I used to do mornings but my schedule changed.",
     "User works out in the evening", "preferences"),
    ("Preference", "User drinks coffee", "User doesn't drink caffeine",
     "caffeine", "I have conflicting info about caffeine — do you drink coffee or not?",
     "I quit caffeine three months ago. Used to be a big coffee drinker.",
     "User does not drink caffeine, quit coffee three months ago", "preferences"),
    ("Preference", "User prefers Android phones", "User recently bought an iPhone",
     "phone_preference", "Seeing a contradiction — you're listed as an Android person but also recently got an iPhone. What's the deal?",
     "Switched to iPhone. Android was fine but the ecosystem integration won me over.",
     "User now uses iPhone, switched from Android", "preferences"),
    ("Temporal", "User's rent is $2000/month (stored October 2025)", "User's rent is $2400/month (stored January 2026)",
     "rent", "I have two different rent amounts — $2000 from October and $2400 from January. Did your rent go up?",
     "Yeah, landlord raised it in January. It's $2400 now.",
     "User's rent is $2400/month as of January 2026", "finance"),
    ("Temporal", "User drives a Honda Civic (stored 2025)", "User drives a Tesla Model 3 (stored Feb 2026)",
     "car", "I have a Honda Civic and a Tesla Model 3 both listed as your car. Did you get a new one?",
     "Got the Tesla in February. Sold the Honda.",
     "User drives a Tesla Model 3, sold Honda Civic", "personal"),
    ("IdentityFact", "User's timezone is EST", "User's timezone is PST",
     "timezone", "I have both EST and PST as your timezone. Which one should I use?",
     "PST. I moved to the west coast last year.",
     "User's timezone is PST", "location"),
    ("Consolidation", "User likes hiking on weekends", "User goes mountain biking on weekends",
     "weekend_activity", "I've got hiking and mountain biking both listed as your weekend activity. Do you do both?",
     "Both actually! I alternate weekends.",
     "User alternates between hiking and mountain biking on weekends", "hobbies"),
    ("Consolidation", "User uses Notion for notes", "User uses Obsidian for notes",
     "note_tool", "Notes app conflict — I have both Notion and Obsidian for you. Using one or both?",
     "Obsidian for personal stuff, Notion for work. Both are accurate.",
     "User uses Obsidian for personal notes and Notion for work notes", "preferences"),
    ("IdentityFact", "User has two siblings", "User has three siblings",
     "family", "Small discrepancy — I have you with either two or three siblings. Which is it?",
     "Three. Two brothers and a sister.",
     "User has three siblings: two brothers and one sister", "family"),
    ("Preference", "User's favorite color is blue", "User's favorite color is green",
     "color", "Got both blue and green as your favorite color. Which one wins?",
     "Green these days. Used to be blue.",
     "User's favorite color is green", "preferences"),
    ("IdentityFact", "User graduated from MIT", "User graduated from Stanford",
     "education", "I have both MIT and Stanford listed as your alma mater. Which did you attend?",
     "MIT for undergrad, Stanford for my masters.",
     "User has a degree from MIT (undergrad) and Stanford (masters)", "education"),
    ("Preference", "User prefers tea", "User drinks coffee every morning",
     "beverage", "Tea or coffee? I've got both listed as your go-to drink.",
     "Coffee in the morning, tea in the afternoon.",
     "User drinks coffee in the morning and tea in the afternoon", "preferences"),
    ("Minor", "User's favorite band is Radiohead", "User's favorite band is Coldplay",
     "music", "Music preference check — Radiohead or Coldplay? I've got both listed as your favorite.",
     "Radiohead by far. I like Coldplay but they're not my favorite.",
     "User's favorite band is Radiohead, also likes Coldplay", "preferences"),
    ("IdentityFact", "User is a cat person", "User recently adopted a dog",
     "pet_preference", "I have you as a cat person but you also adopted a dog. Convert?",
     "I love both now honestly. The dog changed me.",
     "User loves both cats and dogs", "preferences"),
    ("Temporal", "User's salary is $120k (stored June 2025)", "User's salary is $145k (stored Feb 2026)",
     "salary", "I have two salary figures — $120k from June and $145k from February. Did you get a raise?",
     "Yeah, big raise in January. $145k is current.",
     "User's salary is $145k as of January 2026", "finance"),
    ("IdentityFact", "User's email is user@gmail.com", "User's email is user@protonmail.com",
     "email", "I have two emails for you — Gmail and Protonmail. Which is your primary?",
     "Protonmail now. I migrated everything off Gmail for privacy.",
     "User's primary email is Protonmail, migrated from Gmail", "identity"),
    ("Preference", "User sleeps at 11 PM", "User sleeps at 1 AM",
     "sleep_schedule", "Sleep schedule conflict — 11 PM or 1 AM bedtime?",
     "1 AM usually. The 11 PM was aspirational.",
     "User typically sleeps at 1 AM", "routine"),
    ("Consolidation", "User is learning guitar", "User plays piano",
     "instruments", "I have guitar and piano both listed. Are you learning guitar in addition to piano?",
     "Yeah I play piano already and started guitar lessons last month.",
     "User plays piano and is learning guitar", "hobbies"),
]

# ---------------------------------------------------------------------------
# CATEGORY 3: Entity anomaly detection (40 examples)
# ---------------------------------------------------------------------------
# (entity_type, value_a, value_b, field, companion_msg, user_answer, correct_fact, domain)
CAT3_SCENARIOS = [
    ("works_at", "Google", "Meta", "employer",
     "I noticed I have two different companies stored for you — Google and Meta. Which one's right?",
     "Meta. I've been there since October.",
     "User works at Meta since October 2025", "work"),
    ("lives_in", "Boston", "Philadelphia", "city",
     "My records show two different cities for you — Boston and Philadelphia. Where are you based?",
     "Philadelphia. Haven't lived in Boston for two years.",
     "User lives in Philadelphia", "location"),
    ("birthday", "June 5", "June 15", "date",
     "Your records show two different birthdays — June 5 and June 15. Which is correct?",
     "June 5th.",
     "User's birthday is June 5", "identity"),
    ("manager", "Sarah", "Mike", "person",
     "I have two different names for your manager — Sarah and Mike. Who's your manager now?",
     "Sarah since last month. Mike transferred to another team.",
     "User's manager is Sarah, Mike transferred to another team", "work"),
    ("partner", "Alex", "Chris", "person",
     "I've got both Alex and Chris stored as your partner. Can you clarify?",
     "Chris. Alex was my ex.",
     "User's partner is Chris, Alex is an ex", "relationships"),
    ("pet_name", "Luna", "Bella", "name",
     "Two pet names in my records — Luna and Bella. Are these two different pets or a name mix-up?",
     "Luna is our cat and Bella is the dog. Two different pets.",
     "User has a cat named Luna and a dog named Bella", "personal"),
    ("phone_number", "+1-555-0101", "+1-555-0202", "contact",
     "I have two phone numbers on file for you. Did you change your number recently?",
     "The 0202 one is current. I switched carriers last month.",
     "User's current phone number is +1-555-0202", "identity"),
    ("job_title", "Product Manager", "Engineering Manager", "title",
     "I have you listed as both Product Manager and Engineering Manager. Which is your current title?",
     "Engineering Manager. I switched roles in January.",
     "User is an Engineering Manager, switched from Product Manager in January 2026", "work"),
    ("university", "UCLA", "UC Berkeley", "education",
     "Two universities in your records — UCLA and UC Berkeley. Attended both?",
     "UCLA only. I applied to Berkeley but didn't go.",
     "User attended UCLA", "education"),
    ("sibling_name", "James", "Jason", "person",
     "I have your brother's name as both James and Jason. Which is it?",
     "James. No idea where Jason came from.",
     "User's brother's name is James", "family"),
    ("language", "speaks French", "speaks Spanish", "skill",
     "I've got both French and Spanish listed as languages you speak. Both?",
     "Spanish only. I tried French in high school but never got far.",
     "User speaks Spanish, does not speak French", "skills"),
    ("car", "BMW 3 Series", "Audi A4", "vehicle",
     "Two cars in your profile — BMW 3 Series and Audi A4. Which do you drive?",
     "Audi A4. Sold the BMW last year.",
     "User drives an Audi A4, sold the BMW", "personal"),
    ("hometown", "Denver", "Dallas", "city",
     "I have both Denver and Dallas listed as your hometown. Where did you grow up?",
     "Dallas born and raised. I moved to Denver after college.",
     "User's hometown is Dallas, currently lives in Denver", "location"),
    ("team", "Platform Team", "Infrastructure Team", "work_team",
     "I have you on both the Platform Team and Infrastructure Team. Which is current?",
     "They merged. It's called Infrastructure now.",
     "User is on the Infrastructure Team (merged from Platform Team)", "work"),
    ("diet", "vegetarian", "pescatarian", "diet_type",
     "I've got you as both vegetarian and pescatarian. Which is accurate?",
     "Pescatarian. I started eating fish again last year.",
     "User is pescatarian", "preferences"),
    ("gym", "CrossFit box", "Planet Fitness", "fitness",
     "Two gyms in my records — CrossFit and Planet Fitness. Which do you go to?",
     "CrossFit. I cancelled Planet Fitness ages ago.",
     "User goes to CrossFit", "hobbies"),
    ("insurance", "Blue Cross", "Aetna", "health_insurance",
     "I have Blue Cross and Aetna both listed as your health insurance. Which is current?",
     "Aetna now. Changed with my new job.",
     "User has Aetna health insurance", "health"),
    ("dentist", "Dr. Park", "Dr. Williams", "healthcare",
     "Two dentists on file — Dr. Park and Dr. Williams. Who are you seeing?",
     "Dr. Williams. I switched after Dr. Park retired.",
     "User's dentist is Dr. Williams", "health"),
    ("bank", "Chase", "Wells Fargo", "finance",
     "I have both Chase and Wells Fargo as your bank. Using both or just one?",
     "Both actually. Chase for checking, Wells Fargo for savings.",
     "User uses Chase for checking and Wells Fargo for savings", "finance"),
    ("address", "123 Main St", "456 Oak Ave", "home_address",
     "Two addresses in my records — Main Street and Oak Avenue. Which is home?",
     "Oak Avenue. Main Street was my old place.",
     "User lives at 456 Oak Ave", "location"),
    ("best_friend", "Tom", "Jake", "person",
     "I have both Tom and Jake stored as your best friend. Who's your closest friend?",
     "Tom. Jake is a good friend too but Tom's my best mate.",
     "User's best friend is Tom, Jake is also a close friend", "relationships"),
    ("favorite_restaurant", "Sushi Nami", "The Italian Place", "food",
     "Two favorite restaurants — Sushi Nami and The Italian Place. Got a preference?",
     "Sushi Nami these days. Italian Place closed down unfortunately.",
     "User's favorite restaurant is Sushi Nami", "preferences"),
    ("framework", "React", "Vue", "tech",
     "I've got both React and Vue stored as your frontend framework. Which are you using?",
     "React at work, Vue for side projects.",
     "User uses React at work and Vue for personal projects", "tech"),
    ("coffee_shop", "Starbucks", "Blue Bottle", "preferences",
     "Two coffee shops in your profile — Starbucks and Blue Bottle. Regular at one?",
     "Blue Bottle. I avoid Starbucks.",
     "User frequents Blue Bottle, avoids Starbucks", "preferences"),
]

# ---------------------------------------------------------------------------
# CATEGORY 4: Temporal contradictions (30 examples)
# ---------------------------------------------------------------------------
# (old_fact, old_date, check_question, user_response, is_changed, new_fact, domain)
CAT4_SCENARIOS = [
    ("User lives in Portland", "2025-09-01",
     "Last September you mentioned living in Portland. Still there?",
     "Nope, moved to Denver in November.", True,
     "User lives in Denver, moved from Portland November 2025", "location"),
    ("User uses Vue for frontend", "2025-08-15",
     "You told me you were using Vue for the frontend. Is that still the case?",
     "We switched to React. Vue wasn't scaling well.", True,
     "User's team uses React for frontend, switched from Vue", "tech"),
    ("User's commute is 45 minutes", "2026-01-10",
     "I have a note from January that your commute is 45 minutes. Still accurate?",
     "It's about 30 now since they opened the new highway.", True,
     "User's commute is about 30 minutes", "routine"),
    ("User is learning Rust", "2025-10-20",
     "You mentioned learning Rust back in October. Still at it?",
     "Yeah absolutely, getting pretty comfortable with it now.", False,
     None, None),
    ("User goes to gym 4 times a week", "2025-11-05",
     "I have you at the gym 4 times a week from November. Still the routine?",
     "Dropped to 3 times. Work got too busy.", True,
     "User goes to gym 3 times a week", "routine"),
    ("User's team uses Slack", "2025-07-20",
     "Your team was using Slack as of last July. Still the case?",
     "We migrated to Microsoft Teams in December.", True,
     "User's team uses Microsoft Teams, migrated from Slack", "work"),
    ("User reads before bed every night", "2025-09-30",
     "You mentioned reading before bed every night. Still keeping that habit?",
     "Most nights yeah. It's become my favorite routine.", False,
     None, None),
    ("User has a standing desk", "2025-06-15",
     "You had a standing desk last I noted. Still using it?",
     "Actually I switched back to a regular desk. Standing all day was killing my feet.", True,
     "User uses a regular desk, stopped using standing desk", "preferences"),
    ("User takes the bus to work", "2025-08-01",
     "I have you taking the bus to work. Still your commute method?",
     "Got a bike in October. Way faster.", True,
     "User bikes to work, stopped taking the bus", "routine"),
    ("User subscribes to Netflix and Hulu", "2025-10-12",
     "You had Netflix and Hulu subscriptions back in October. Still have both?",
     "Dropped Hulu. Netflix and Disney+ now.", True,
     "User subscribes to Netflix and Disney+, cancelled Hulu", "preferences"),
    ("User plays tennis on Sundays", "2025-07-08",
     "Sunday tennis — still a thing?",
     "Every week without fail.", False,
     None, None),
    ("User's team size is 8 people", "2025-11-20",
     "Your team was 8 people last November. Same size?",
     "We hired three more, so 11 now.", True,
     "User's team has 11 people", "work"),
    ("User is training for a marathon", "2025-12-01",
     "You were training for a marathon in December. How's that going?",
     "Ran it last month! Finished in 4 hours 12 minutes.", True,
     "User completed a marathon in 4:12", "hobbies"),
    ("User uses a MacBook Pro", "2025-09-15",
     "Still on the MacBook Pro?",
     "Switched to a Framework laptop. Love it.", True,
     "User uses a Framework laptop, switched from MacBook Pro", "tech"),
    ("User meditates every morning", "2025-10-01",
     "Still doing morning meditation?",
     "Honestly I fell off that habit months ago.", True,
     "User stopped meditating regularly", "routine"),
    ("User's favorite podcast is Lex Fridman", "2025-08-20",
     "Still listening to Lex Fridman?",
     "Every episode. He had an amazing guest last week.", False,
     None, None),
    ("User drinks green smoothies daily", "2025-11-10",
     "Still on the daily green smoothie kick?",
     "Switched to just coffee. The smoothie phase ended.", True,
     "User no longer drinks daily green smoothies", "preferences"),
    ("User's rent is $1800/month", "2025-06-01",
     "Your rent was $1800 back in June. Same?",
     "Went up to $2100 when I renewed the lease.", True,
     "User's rent is $2100/month", "finance"),
    ("User works remote full time", "2025-09-01",
     "Last I noted you were fully remote. Still working from home?",
     "Hybrid now. Three days in office, two remote.", True,
     "User works hybrid — 3 days office, 2 days remote", "work"),
    ("User's dog is a puppy", "2025-03-01",
     "Your pup must be about a year old now. How's the dog?",
     "Officially not a puppy anymore! Still acts like one though.", True,
     "User's dog is about 1 year old", "personal"),
]

# ---------------------------------------------------------------------------
# CATEGORY 5: User-initiated conflict resolution (30 examples)
# ---------------------------------------------------------------------------
# (user_complaint, recall_query, found_memories, companion_question, user_answer,
#  correct_fact, domain, wrong_rid)
CAT5_SCENARIOS = [
    ("You keep getting my timezone wrong.",
     "timezone location time zone",
     [{"rid": "mem_tz01", "text": "User's timezone is EST", "domain": "location", "date": "2025-09-01"},
      {"rid": "mem_tz02", "text": "User is in PST", "domain": "location", "date": "2026-01-15"}],
     "I see the issue — I have both EST and PST stored. Which is right?",
     "PST. I moved to California in December.",
     "User's timezone is PST, located in California", "location", "mem_tz01"),
    ("You said two different things about my sister's name.",
     "sister sibling family name",
     [{"rid": "mem_sis01", "text": "User's sister is named Emma", "domain": "family", "date": "2025-07-20"},
      {"rid": "mem_sis02", "text": "User's sister's name is Emily", "domain": "family", "date": "2025-11-03"}],
     "You're right, I have both Emma and Emily. Which one is your sister?",
     "Emily. Must have been autocorrect the first time.",
     "User's sister's name is Emily", "family", "mem_sis01"),
    ("Your memories about my work schedule are all wrong.",
     "work schedule hours routine",
     [{"rid": "mem_ws01", "text": "User works 9 to 5", "domain": "routine", "date": "2025-08-10"},
      {"rid": "mem_ws02", "text": "User works 10 to 6", "domain": "routine", "date": "2025-12-01"},
      {"rid": "mem_ws03", "text": "User works night shift", "domain": "routine", "date": "2026-02-15"}],
     "I've got three different schedules: 9-5, 10-6, and night shift. What's current?",
     "10 to 6 is correct. I've never worked nights.",
     "User works 10 AM to 6 PM", "routine", "mem_ws01"),
    ("You have the wrong birthday for me.",
     "birthday birth date born",
     [{"rid": "mem_bd01", "text": "User's birthday is April 20", "domain": "identity", "date": "2025-06-15"},
      {"rid": "mem_bd02", "text": "User's birthday is April 22", "domain": "identity", "date": "2025-09-30"}],
     "I have April 20 and April 22. What's the real date?",
     "April 22nd.",
     "User's birthday is April 22", "identity", "mem_bd01"),
    ("Why do you think I live in two different cities?",
     "location city home address lives",
     [{"rid": "mem_loc01", "text": "User lives in Miami", "domain": "location", "date": "2025-05-20"},
      {"rid": "mem_loc02", "text": "User lives in Atlanta", "domain": "location", "date": "2026-01-10"}],
     "Good catch — I have both Miami and Atlanta. Where are you actually based?",
     "Atlanta. Moved from Miami last year.",
     "User lives in Atlanta, moved from Miami", "location", "mem_loc01"),
    ("You have conflicting info about my diet.",
     "diet food eating preferences",
     [{"rid": "mem_diet01", "text": "User is vegan", "domain": "preferences", "date": "2025-08-05"},
      {"rid": "mem_diet02", "text": "User eats meat", "domain": "preferences", "date": "2026-02-20"}],
     "Yeah I see it — vegan and meat-eater. What's the current situation?",
     "I eat meat now. Was vegan for a while but stopped.",
     "User eats meat, previously was vegan", "preferences", "mem_diet01"),
    ("Stop telling me about my running schedule, I already told you I switched to swimming.",
     "exercise running swimming workout",
     [{"rid": "mem_ex01", "text": "User runs 5K every morning", "domain": "hobbies", "date": "2025-07-15"},
      {"rid": "mem_ex02", "text": "User swims 3 times a week", "domain": "hobbies", "date": "2026-01-20"}],
     "My bad — I still had the running note. I'll clean that up. Swimming 3 times a week, right?",
     "Exactly. No more running.",
     "User swims 3 times a week, no longer runs", "hobbies", "mem_ex01"),
    ("You keep mixing up my girlfriend's name.",
     "girlfriend partner relationship name",
     [{"rid": "mem_gf01", "text": "User's girlfriend is named Sophia", "domain": "relationships", "date": "2025-10-01"},
      {"rid": "mem_gf02", "text": "User's girlfriend is named Sophie", "domain": "relationships", "date": "2025-12-15"}],
     "I have Sophia and Sophie — are those the same person or different?",
     "Same person, she goes by Sophie.",
     "User's girlfriend's name is Sophie", "relationships", "mem_gf01"),
    ("You have wrong information about my car.",
     "car vehicle drive automobile",
     [{"rid": "mem_car01", "text": "User drives a Toyota Camry", "domain": "personal", "date": "2025-06-10"},
      {"rid": "mem_car02", "text": "User drives a Honda Accord", "domain": "personal", "date": "2026-02-01"}],
     "I've got Toyota Camry and Honda Accord. Which do you drive?",
     "Honda Accord. Traded in the Toyota.",
     "User drives a Honda Accord, traded in Toyota Camry", "personal", "mem_car01"),
    ("My allergies are listed wrong. I told you I'm not allergic to peanuts.",
     "allergy allergies food medical",
     [{"rid": "mem_al01", "text": "User is allergic to peanuts", "domain": "health", "date": "2025-09-01"},
      {"rid": "mem_al02", "text": "User has no food allergies", "domain": "health", "date": "2026-01-05"}],
     "I see the conflict — peanut allergy vs no allergies. No food allergies at all?",
     "None. The peanut thing was a misunderstanding.",
     "User has no food allergies", "health", "mem_al01"),
    ("You keep confusing my brother and my cousin.",
     "brother cousin family sibling",
     [{"rid": "mem_fam01", "text": "User's brother is named Rahul", "domain": "family", "date": "2025-08-20"},
      {"rid": "mem_fam02", "text": "User's cousin is named Rahul", "domain": "family", "date": "2025-11-10"}],
     "I have Rahul listed as both your brother and your cousin. Which is it?",
     "Rahul is my cousin. My brother's name is Arjun.",
     "Rahul is user's cousin, Arjun is user's brother", "family", "mem_fam01"),
    ("Your notes say I work in two different departments.",
     "department team work group",
     [{"rid": "mem_dep01", "text": "User works in Marketing", "domain": "work", "date": "2025-07-01"},
      {"rid": "mem_dep02", "text": "User works in Sales", "domain": "work", "date": "2025-12-20"}],
     "Marketing and Sales — which department are you in?",
     "Sales. I transferred from Marketing six months ago.",
     "User works in Sales, transferred from Marketing", "work", "mem_dep01"),
    ("You gave me directions in miles last time but I use kilometers.",
     "units metric imperial measurement",
     [{"rid": "mem_u01", "text": "User prefers imperial units", "domain": "preferences", "date": "2025-06-15"},
      {"rid": "mem_u02", "text": "User uses metric system", "domain": "preferences", "date": "2026-01-30"}],
     "Got it — I had both imperial and metric stored. You prefer metric?",
     "Yes, always metric. I'm Canadian.",
     "User prefers metric system (Canadian)", "preferences", "mem_u01"),
    ("You seem confused about what language I code in.",
     "programming language code development",
     [{"rid": "mem_pl01", "text": "User codes in Python", "domain": "tech", "date": "2025-08-10"},
      {"rid": "mem_pl02", "text": "User codes in Go", "domain": "tech", "date": "2025-12-05"}],
     "I have Python and Go both listed. What's your main language?",
     "Go for the past six months. I still know Python but Go is my primary now.",
     "User's primary language is Go, also knows Python", "tech", "mem_pl01"),
    ("I never said I have a cat. Why do you keep mentioning my cat?",
     "cat pet animal",
     [{"rid": "mem_cat01", "text": "User has a cat named Whiskers", "domain": "personal", "date": "2025-10-15"}],
     "My apologies — I have a note that you have a cat named Whiskers. Is that wrong?",
     "Completely wrong. I only have a dog.",
     "User does not have a cat, has a dog only", "personal", "mem_cat01"),
]


# ---------------------------------------------------------------------------
# Generation helpers
# ---------------------------------------------------------------------------

def pick_params():
    name = random.choice(NAMES)
    bond = random.choice(BOND_STAGES)
    time = random.choice(TIMES)
    return name, bond, time


def gen_category1(scenarios):
    """New info contradicts stored memory — companion catches it."""
    examples = []
    for s in scenarios:
        (user_msg, recall_query, old_mem_text, old_domain, old_date,
         companion_q, user_ans, new_fact, new_domain, forget_reason) = s
        name, bond, time = pick_params()
        old_rid = mem_id()

        memories = [{"rid": old_rid, "text": old_mem_text, "domain": old_domain, "date": old_date}]
        sys_prompt = make_system_prompt(name, bond, time, memories=memories)

        # Turn 1: user says something contradictory
        # Turn 2: companion recalls to check
        tc_recall = make_tool_call("recall", {"query": recall_query})
        recall_result = {
            "status": "ok",
            "memories": [{"rid": old_rid, "text": old_mem_text, "domain": old_domain,
                          "importance": 0.8, "created": old_date}]
        }
        # Turn 3: companion asks about contradiction
        # Turn 4: user confirms change
        # Turn 5: companion saves new fact + forgets old
        tc_save = make_tool_call("save_user_fact", {"fact": new_fact, "domain": new_domain, "confidence": 0.95})
        tc_forget = make_tool_call("forget_memory", {"rid": old_rid, "reason": forget_reason})
        save_result = {"status": "ok", "rid": mem_id()}
        forget_result = {"status": "ok", "forgotten": old_rid}

        # Tone-adjust the question
        if bond == "partner_in_crime":
            companion_q_adj = companion_q.replace("should I update that?", "want me to fix that?").replace(
                "I had you down as", "I had you as")
        elif bond == "stranger":
            companion_q_adj = companion_q.replace("Wait, ", "If I may ask, ")
        else:
            companion_q_adj = companion_q

        confirmation = random.choice([
            "Got it, updated.", "Done, I've updated my records.", "All set.", "Updated.",
            "Noted, I've made the change.", "Fixed. Your records are up to date now.",
        ])

        conversations = [
            {"role": "system", "content": sys_prompt},
            {"role": "user", "content": user_msg},
            {"role": "assistant", "content": None, "tool_calls": [tc_recall]},
            {"role": "tool", "content": json.dumps(recall_result), "tool_call_id": tc_recall["id"]},
            {"role": "assistant", "content": companion_q_adj},
            {"role": "user", "content": user_ans},
            {"role": "assistant", "content": None, "tool_calls": [tc_save, tc_forget]},
            {"role": "tool", "content": json.dumps(save_result), "tool_call_id": tc_save["id"]},
            {"role": "tool", "content": json.dumps(forget_result), "tool_call_id": tc_forget["id"]},
            {"role": "assistant", "content": confirmation},
        ]
        examples.append({
            "conversations": conversations,
            "metadata": {"bond_stage": bond, "scenario_type": "contradiction_catch"},
        })
    return examples


def gen_category2(scenarios):
    """Proactive conflict surfacing — companion brings up detected conflicts."""
    examples = []
    for s in scenarios:
        (conflict_type, mem_a, mem_b, entity, companion_intro,
         user_resolution, resolved_fact, domain) = s
        name, bond, time = pick_params()
        conflict_id = f"conf_{uuid.uuid4().hex[:8]}"
        rid_a = mem_id()
        rid_b = mem_id()

        open_conflicts = [{
            "type": conflict_type, "mem_a": mem_a, "mem_b": mem_b,
            "entity": entity, "id": conflict_id,
            "date": random.choice(["2026-03-12", "2026-03-11", "2026-03-10"]),
        }]
        sys_prompt = make_system_prompt(name, bond, time, open_conflicts=open_conflicts)

        # Companion proactively raises the conflict
        # User resolves it
        # Companion saves correct fact + resolves conflict + forgets wrong memory
        tc_resolve = make_tool_call("resolve_conflicts",
                                     {"action": "resolve_one", "conflict_id": conflict_id})
        tc_save = make_tool_call("save_user_fact",
                                  {"fact": resolved_fact, "domain": domain, "confidence": 0.95})
        resolve_result = {"status": "ok", "resolved": conflict_id}
        save_result = {"status": "ok", "rid": mem_id()}

        if bond == "partner_in_crime":
            intro_adj = companion_intro.replace("Quick thing", "Yo, quick thing").replace(
                "I have conflicting", "I've got conflicting")
        elif bond == "stranger":
            intro_adj = "I wanted to check on something. " + companion_intro
        else:
            intro_adj = companion_intro

        confirmation = random.choice([
            "Got it, conflict resolved.", "All clear, I've sorted that out.",
            "Done. Cleaned up the conflicting notes.", "Fixed. My records are consistent now.",
            "Updated and resolved.",
        ])

        conversations = [
            {"role": "system", "content": sys_prompt},
            # Companion initiates — this is a proactive message, user hasn't said anything specific
            {"role": "user", "content": random.choice([
                "Hey", "What's up?", "Hi", "Morning", "Good afternoon",
                "Hey there", "Yo", "Sup", "Hello"])},
            {"role": "assistant", "content": intro_adj},
            {"role": "user", "content": user_resolution},
            {"role": "assistant", "content": None, "tool_calls": [tc_resolve, tc_save]},
            {"role": "tool", "content": json.dumps(resolve_result), "tool_call_id": tc_resolve["id"]},
            {"role": "tool", "content": json.dumps(save_result), "tool_call_id": tc_save["id"]},
            {"role": "assistant", "content": confirmation},
        ]
        examples.append({
            "conversations": conversations,
            "metadata": {"bond_stage": bond, "scenario_type": "proactive_conflict"},
        })
    return examples


def gen_category3(scenarios):
    """Entity anomaly detection — same entity, multiple values."""
    examples = []
    for s in scenarios:
        (entity_type, value_a, value_b, field, companion_msg,
         user_answer, correct_fact, domain) = s
        name, bond, time = pick_params()
        rid_a = mem_id()
        rid_b = mem_id()

        memories = [
            {"rid": rid_a, "text": f"User {entity_type}: {value_a}", "domain": domain,
             "date": random.choice(["2025-07-10", "2025-08-20", "2025-09-15"])},
            {"rid": rid_b, "text": f"User {entity_type}: {value_b}", "domain": domain,
             "date": random.choice(["2025-12-01", "2026-01-15", "2026-02-10"])},
        ]
        sys_prompt = make_system_prompt(name, bond, time, memories=memories)

        # Determine which to keep (heuristic: second one is newer, but user decides)
        # We need a recall, then ask, then update
        tc_recall = make_tool_call("recall", {"query": f"{entity_type} {field}"})
        recall_result = {
            "status": "ok",
            "memories": [
                {"rid": rid_a, "text": f"User {entity_type}: {value_a}", "domain": domain,
                 "importance": 0.8, "created": memories[0]["date"]},
                {"rid": rid_b, "text": f"User {entity_type}: {value_b}", "domain": domain,
                 "importance": 0.8, "created": memories[1]["date"]},
            ]
        }

        # After user answers, save correct fact and forget wrong one
        # Determine which rid to forget — if both values used, keep both, else forget old
        if value_a.lower() in correct_fact.lower() and value_b.lower() in correct_fact.lower():
            # Both kept — use relate_entities instead of forget
            tc_save = make_tool_call("save_user_fact",
                                      {"fact": correct_fact, "domain": domain, "confidence": 0.95})
            tc_relate = make_tool_call("relate_entities",
                                        {"source": "user", "target": value_a, "relationship": entity_type})
            save_result = {"status": "ok", "rid": mem_id()}
            relate_result = {"status": "ok"}

            confirmation = random.choice([
                "Got it, both noted.", "Makes sense. I've updated my records.",
                "Understood, I've got both down now.", "All sorted.",
            ])

            conversations = [
                {"role": "system", "content": sys_prompt},
                {"role": "user", "content": random.choice(["Hey", "Hi", "What's new?"])},
                {"role": "assistant", "content": None, "tool_calls": [tc_recall]},
                {"role": "tool", "content": json.dumps(recall_result), "tool_call_id": tc_recall["id"]},
                {"role": "assistant", "content": companion_msg},
                {"role": "user", "content": user_answer},
                {"role": "assistant", "content": None, "tool_calls": [tc_save, tc_relate]},
                {"role": "tool", "content": json.dumps(save_result), "tool_call_id": tc_save["id"]},
                {"role": "tool", "content": json.dumps(relate_result), "tool_call_id": tc_relate["id"]},
                {"role": "assistant", "content": confirmation},
            ]
        else:
            # One is wrong — forget it
            wrong_rid = rid_a  # default: older one is wrong
            tc_save = make_tool_call("save_user_fact",
                                      {"fact": correct_fact, "domain": domain, "confidence": 0.95})
            tc_forget = make_tool_call("forget_memory",
                                        {"rid": wrong_rid, "reason": f"Incorrect {entity_type} — user clarified"})
            save_result = {"status": "ok", "rid": mem_id()}
            forget_result = {"status": "ok", "forgotten": wrong_rid}

            confirmation = random.choice([
                "Got it, fixed.", "Updated. The old entry is removed.",
                "All set, cleaned that up.", "Done, I've corrected it.",
            ])

            conversations = [
                {"role": "system", "content": sys_prompt},
                {"role": "user", "content": random.choice(["Hey", "Hi", "What's new?"])},
                {"role": "assistant", "content": None, "tool_calls": [tc_recall]},
                {"role": "tool", "content": json.dumps(recall_result), "tool_call_id": tc_recall["id"]},
                {"role": "assistant", "content": companion_msg},
                {"role": "user", "content": user_answer},
                {"role": "assistant", "content": None, "tool_calls": [tc_save, tc_forget]},
                {"role": "tool", "content": json.dumps(save_result), "tool_call_id": tc_save["id"]},
                {"role": "tool", "content": json.dumps(forget_result), "tool_call_id": tc_forget["id"]},
                {"role": "assistant", "content": confirmation},
            ]

        examples.append({
            "conversations": conversations,
            "metadata": {"bond_stage": bond, "scenario_type": "entity_anomaly"},
        })
    return examples


def gen_category4(scenarios):
    """Temporal contradictions — is this still true?"""
    examples = []
    for s in scenarios:
        (old_fact, old_date, check_question, user_response,
         is_changed, new_fact, domain) = s
        name, bond, time = pick_params()
        old_rid = mem_id()

        memories = [{"rid": old_rid, "text": old_fact, "domain": domain or "general", "date": old_date}]
        sys_prompt = make_system_prompt(name, bond, time, memories=memories)

        if is_changed:
            tc_save = make_tool_call("save_user_fact",
                                      {"fact": new_fact, "domain": domain, "confidence": 0.9})
            tc_update = make_tool_call("update_memory",
                                        {"rid": old_rid, "importance": 0.2, "domain": domain})
            save_result = {"status": "ok", "rid": mem_id()}
            update_result = {"status": "ok", "updated": old_rid}

            confirmation = random.choice([
                "Got it, updated.", "Noted the change.", "All set, I've updated that.",
                "Fixed. Thanks for letting me know.", "Updated my records.",
            ])

            conversations = [
                {"role": "system", "content": sys_prompt},
                {"role": "user", "content": random.choice(["Hey", "Hi", "What's up?"])},
                {"role": "assistant", "content": check_question},
                {"role": "user", "content": user_response},
                {"role": "assistant", "content": None, "tool_calls": [tc_save, tc_update]},
                {"role": "tool", "content": json.dumps(save_result), "tool_call_id": tc_save["id"]},
                {"role": "tool", "content": json.dumps(update_result), "tool_call_id": tc_update["id"]},
                {"role": "assistant", "content": confirmation},
            ]
        else:
            # Still true — no tool call needed, just acknowledge
            confirmation = random.choice([
                "Good to know it's still the same.", "Great, no update needed then.",
                "Cool, noted that it's still the case.", "All good, keeping that as is.",
            ])
            conversations = [
                {"role": "system", "content": sys_prompt},
                {"role": "user", "content": random.choice(["Hey", "Hi", "What's up?"])},
                {"role": "assistant", "content": check_question},
                {"role": "user", "content": user_response},
                {"role": "assistant", "content": confirmation},
            ]

        examples.append({
            "conversations": conversations,
            "metadata": {"bond_stage": bond, "scenario_type": "temporal_check"},
        })
    return examples


def gen_category5(scenarios):
    """User-initiated conflict resolution."""
    examples = []
    for s in scenarios:
        (user_complaint, recall_query, found_memories, companion_question,
         user_answer, correct_fact, domain, wrong_rid) = s
        name, bond, time = pick_params()

        # Build system prompt with the conflicting memories
        memories_for_prompt = [
            {"rid": m["rid"], "text": m["text"], "domain": m["domain"], "date": m["date"]}
            for m in found_memories
        ]
        sys_prompt = make_system_prompt(name, bond, time, memories=memories_for_prompt)

        # Turn 1: user complains
        # Turn 2: companion recalls
        tc_recall = make_tool_call("recall", {"query": recall_query})
        recall_result = {
            "status": "ok",
            "memories": [{"rid": m["rid"], "text": m["text"], "domain": m["domain"],
                          "importance": 0.7, "created": m["date"]} for m in found_memories]
        }
        # Turn 3: companion asks for clarification
        # Turn 4: user clarifies
        # Turn 5: companion saves correct + forgets wrong
        tc_save = make_tool_call("save_user_fact",
                                  {"fact": correct_fact, "domain": domain, "confidence": 0.95})
        save_result = {"status": "ok", "rid": mem_id()}

        tool_calls_resolve = [tc_save]
        tool_results = [
            {"role": "tool", "content": json.dumps(save_result), "tool_call_id": tc_save["id"]},
        ]

        # Forget the wrong memories
        if isinstance(wrong_rid, list):
            for wr in wrong_rid:
                tc_f = make_tool_call("forget_memory", {"rid": wr, "reason": "Incorrect — user clarified"})
                tool_calls_resolve.append(tc_f)
                tool_results.append(
                    {"role": "tool", "content": json.dumps({"status": "ok", "forgotten": wr}),
                     "tool_call_id": tc_f["id"]})
        else:
            tc_f = make_tool_call("forget_memory",
                                   {"rid": wrong_rid, "reason": "Incorrect — user clarified"})
            tool_calls_resolve.append(tc_f)
            tool_results.append(
                {"role": "tool", "content": json.dumps({"status": "ok", "forgotten": wrong_rid}),
                 "tool_call_id": tc_f["id"]})

        if bond == "partner_in_crime":
            apology = random.choice([
                "My bad. Cleaned that up.", "Oops, fixed.", "Sorry about that. All sorted now.",
            ])
        elif bond == "stranger":
            apology = random.choice([
                "I apologize for the confusion. I've corrected my records.",
                "Sorry about that. It's been fixed now.",
                "My apologies. The records are updated.",
            ])
        else:
            apology = random.choice([
                "Sorry about the mix-up. Fixed now.", "My bad, that's cleaned up.",
                "Got it. Sorry for the confusion — all fixed.",
                "Apologies. I've sorted it out.",
            ])

        conversations = [
            {"role": "system", "content": sys_prompt},
            {"role": "user", "content": user_complaint},
            {"role": "assistant", "content": None, "tool_calls": [tc_recall]},
            {"role": "tool", "content": json.dumps(recall_result), "tool_call_id": tc_recall["id"]},
            {"role": "assistant", "content": companion_question},
            {"role": "user", "content": user_answer},
            {"role": "assistant", "content": None, "tool_calls": tool_calls_resolve},
        ] + tool_results + [
            {"role": "assistant", "content": apology},
        ]

        examples.append({
            "conversations": conversations,
            "metadata": {"bond_stage": bond, "scenario_type": "user_initiated_resolution"},
        })
    return examples


# ---------------------------------------------------------------------------
# Augmentation: duplicate scenarios with variations to reach target counts
# ---------------------------------------------------------------------------

def augment_scenarios(base_scenarios, target_count, mutator_fn):
    """Repeat base scenarios with minor variations to reach target count."""
    result = list(base_scenarios)
    idx = 0
    while len(result) < target_count:
        s = base_scenarios[idx % len(base_scenarios)]
        result.append(mutator_fn(s))
        idx += 1
    return result[:target_count]


def mutate_cat1(s):
    """Swap to a slightly different phrasing / context."""
    s_list = list(s)
    # Vary the user message slightly
    prefixes = ["So, ", "Hey, ", "By the way, ", "Oh, ", "Just so you know, ", "Update: ", "FYI — "]
    s_list[0] = random.choice(prefixes) + s_list[0][0].lower() + s_list[0][1:]
    return tuple(s_list)


def mutate_cat2(s):
    s_list = list(s)
    # Vary intro phrasing
    intros = [
        "Quick question — ", "Hey, small thing — ", "By the way, ", "Before I forget — ",
        "Oh, I wanted to ask — ", "Something I need to sort out — ",
    ]
    s_list[4] = random.choice(intros) + s_list[4][0].lower() + s_list[4][1:]
    return tuple(s_list)


def mutate_cat3(s):
    s_list = list(s)
    intros = [
        "Small discrepancy in my records — ", "Quick data check — ",
        "I noticed something conflicting — ", "Need to sort this out — ",
    ]
    s_list[4] = random.choice(intros) + s_list[4][0].lower() + s_list[4][1:]
    return tuple(s_list)


def mutate_cat4(s):
    s_list = list(s)
    # Vary the check question phrasing
    prefixes = ["Just checking — ", "Quick update check — ", "Is this still accurate: ",
                "Wanted to verify — "]
    s_list[2] = random.choice(prefixes) + s_list[2][0].lower() + s_list[2][1:]
    return tuple(s_list)


def mutate_cat5(s):
    s_list = list(s)
    complaints = [
        "Your info about this is wrong. ", "This is incorrect in your memory. ",
        "Can you fix this? ", "Please update this — ",
    ]
    s_list[0] = random.choice(complaints) + s_list[0]
    return tuple(s_list)


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main():
    # Target: 60 + 40 + 40 + 30 + 30 = 200
    cat1_scenarios = augment_scenarios(CAT1_SCENARIOS, 60, mutate_cat1)
    cat2_scenarios = augment_scenarios(CAT2_SCENARIOS, 40, mutate_cat2)
    cat3_scenarios = augment_scenarios(CAT3_SCENARIOS, 40, mutate_cat3)
    cat4_scenarios = augment_scenarios(CAT4_SCENARIOS, 30, mutate_cat4)
    cat5_scenarios = augment_scenarios(CAT5_SCENARIOS, 30, mutate_cat5)

    all_examples = []
    all_examples.extend(gen_category1(cat1_scenarios))
    all_examples.extend(gen_category2(cat2_scenarios))
    all_examples.extend(gen_category3(cat3_scenarios))
    all_examples.extend(gen_category4(cat4_scenarios))
    all_examples.extend(gen_category5(cat5_scenarios))

    random.shuffle(all_examples)

    out_path = os.path.join(os.path.dirname(__file__), "batch_memory_04_conflicts.jsonl")
    with open(out_path, "w", encoding="utf-8") as f:
        for ex in all_examples:
            f.write(json.dumps(ex, ensure_ascii=False) + "\n")

    print(f"Wrote {len(all_examples)} examples to {out_path}")

    # Verify category counts
    counts = {}
    for ex in all_examples:
        st = ex["metadata"]["scenario_type"]
        counts[st] = counts.get(st, 0) + 1
    for k, v in sorted(counts.items()):
        print(f"  {k}: {v}")


if __name__ == "__main__":
    main()
