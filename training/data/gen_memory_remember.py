#!/usr/bin/env python3
"""Generate 400 JSONL training examples for proactive remember/save_user_fact tool usage."""

import json
import random
import uuid
import os

random.seed(73)

NAMES = ["Alex", "Sam", "Maya", "Jordan", "Priya", "Leo", "Kai", "Noor", "Tess", "Ravi",
         "Zoe", "Finn", "Anya", "Cole", "Devi"]

BOND_STAGES = ["stranger", "acquaintance", "trusted", "deep"]

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
}

TIMES = [
    "2026-03-13 08:15 AM", "2026-03-13 09:30 AM", "2026-03-13 11:45 AM",
    "2026-03-13 01:20 PM", "2026-03-13 03:00 PM", "2026-03-13 05:30 PM",
    "2026-03-13 07:15 PM", "2026-03-13 09:45 PM", "2026-03-12 10:00 AM",
    "2026-03-11 02:30 PM", "2026-03-10 08:00 AM", "2026-03-09 06:00 PM",
]


def make_system_prompt(name, bond_stage, time):
    return (
        f"You are {name}, the user's personal companion. "
        f"{BOND_INSTRUCTIONS[bond_stage]} "
        f"You have persistent memory. When the user shares personal information, preferences, "
        f"or important events, use the remember or save_user_fact tool to store it. "
        f"Personal facts deserve save_user_fact (longer retention). "
        f"Events and observations use remember.\n\n"
        f"Current time: {time}.\n"
        f"Tools available: recall, remember, save_user_fact, set_reminder, forget_topic"
    )


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


def make_example(user_msg, tool_name, tool_args, tool_result, assistant_response,
                 bond_stage, scenario_type, name=None, time=None):
    name = name or random.choice(NAMES)
    time = time or random.choice(TIMES)
    bond = bond_stage or random.choice(BOND_STAGES)
    sys_prompt = make_system_prompt(name, bond, time)

    tc = make_tool_call(tool_name, tool_args)
    conversations = [
        {"role": "system", "content": sys_prompt},
        {"role": "user", "content": user_msg},
        {"role": "assistant", "content": None, "tool_calls": [tc]},
        {"role": "tool", "content": json.dumps(tool_result), "tool_call_id": tc["id"]},
        {"role": "assistant", "content": assistant_response},
    ]
    return {
        "conversations": conversations,
        "metadata": {"bond_stage": bond, "scenario_type": scenario_type},
    }


def make_no_tool_example(user_msg, assistant_response, bond_stage, scenario_type,
                         name=None, time=None):
    name = name or random.choice(NAMES)
    time = time or random.choice(TIMES)
    bond = bond_stage or random.choice(BOND_STAGES)
    sys_prompt = make_system_prompt(name, bond, time)

    conversations = [
        {"role": "system", "content": sys_prompt},
        {"role": "user", "content": user_msg},
        {"role": "assistant", "content": assistant_response},
    ]
    return {
        "conversations": conversations,
        "metadata": {"bond_stage": bond, "scenario_type": scenario_type},
    }


# ---------------------------------------------------------------------------
# CATEGORY 1: Casual mention of personal facts (120 examples)
# ---------------------------------------------------------------------------
CASUAL_FACTS = [
    # (user_msg, fact, domain, confidence, response)
    ("I just moved to Portland last month.", "User moved to Portland last month", "location", 0.95, "Portland's great. Hope the move went smoothly."),
    ("My daughter starts kindergarten in September.", "User has a daughter starting kindergarten September 2026", "family", 0.9, "That's a big milestone. She must be excited."),
    ("I'm vegetarian by the way.", "User is vegetarian", "preference", 0.95, "Good to know. I'll keep that in mind."),
    ("I work at Google on the search team.", "User works at Google on the search team", "work", 0.95, "That's a fascinating team to be on."),
    ("My birthday is March 15th.", "User's birthday is March 15", "identity", 1.0, "Noted. That's coming up soon."),
    ("I've been learning Rust for about six months.", "User is learning Rust, started around September 2025", "hobby", 0.85, "Six months in is when things start clicking. How's it going?"),
    ("I'm originally from Mumbai.", "User is originally from Mumbai", "identity", 0.95, "Mumbai's an incredible city. Do you get back often?"),
    ("My wife is a doctor.", "User's wife is a doctor", "family", 0.9, "That's a demanding but rewarding profession."),
    ("I have two cats, Luna and Mochi.", "User has two cats named Luna and Mochi", "family", 0.9, "Luna and Mochi. Good names."),
    ("I graduated from MIT in 2018.", "User graduated from MIT in 2018", "identity", 0.95, "Solid background."),
    ("I'm lactose intolerant.", "User is lactose intolerant", "health", 0.95, "I'll remember that for any food-related suggestions."),
    ("My name is actually Aleksandr, but everyone calls me Alex.", "User's full name is Aleksandr, goes by Alex", "identity", 1.0, "Got it, Alex."),
    ("I drive a Tesla Model 3.", "User drives a Tesla Model 3", "general", 0.9, "Nice ride. The Model 3 is solid."),
    ("We just bought a house in the suburbs.", "User recently bought a house in the suburbs", "location", 0.9, "Congrats on the house. Big step."),
    ("I'm training for a marathon.", "User is training for a marathon", "hobby", 0.85, "That takes serious commitment. When's the race?"),
    ("English is my second language. I grew up speaking Korean.", "User's first language is Korean, English is second", "identity", 0.95, "Your English is excellent. Korean's a beautiful language."),
    ("I work remotely full time.", "User works remotely full time", "work", 0.9, "Remote life has its perks. Been doing it long?"),
    ("I have a peanut allergy, it's pretty severe.", "User has a severe peanut allergy", "health", 1.0, "That's important to know. I'll flag anything with peanuts."),
    ("My son just turned three.", "User has a three-year-old son", "family", 0.9, "Three is such a fun age. Exhausting, but fun."),
    ("I'm left-handed.", "User is left-handed", "identity", 0.9, "Lefties represent."),
    ("My company does about 50 million in revenue.", "User's company does approximately 50 million in revenue", "work", 0.85, "That's a solid business."),
    ("I play piano. Have been since I was seven.", "User plays piano, started at age seven", "hobby", 0.9, "That's a long time. You must be quite good."),
    ("I'm a morning person. Always up by 5:30.", "User is a morning person, typically up by 5:30 AM", "preference", 0.9, "5:30 is early. That gives you a head start on the day."),
    ("We have a golden retriever named Max.", "User has a golden retriever named Max", "family", 0.9, "Max the golden. Classic combo."),
    ("I'm in my mid-30s.", "User is in their mid-30s", "identity", 0.8, "Prime time."),
    ("I moved from New York to Austin two years ago.", "User moved from New York to Austin about two years ago", "location", 0.9, "Big change from New York. How are you liking Austin?"),
    ("My brother works at the same company as me.", "User's brother works at the same company", "family", 0.85, "That's either great or complicated. Maybe both."),
    ("I'm a type 1 diabetic.", "User has type 1 diabetes", "health", 1.0, "Important to know. I'll keep that in mind."),
    ("I used to be a chef before I got into tech.", "User was previously a chef before transitioning to tech", "identity", 0.9, "That's a unique background. The attention to detail probably carries over."),
    ("I just got promoted to senior engineer.", "User was recently promoted to senior engineer", "work", 0.9, "Congratulations. Well deserved, I'm sure."),
    ("My parents live in Florida.", "User's parents live in Florida", "family", 0.9, "Do you visit them often?"),
    ("I've been at this company for eight years.", "User has been at their company for eight years", "work", 0.9, "Eight years is serious tenure these days."),
    ("My go-to programming language is Python.", "User's primary programming language is Python", "work", 0.9, "Python's a great choice. Versatile."),
    ("I take blood pressure medication daily.", "User takes daily blood pressure medication", "health", 0.95, "Noted. That's good to keep track of."),
    ("We're expecting a baby in July.", "User is expecting a baby in July 2026", "family", 0.95, "That's wonderful news. Congratulations."),
    ("I speak three languages: English, Spanish, and French.", "User speaks English, Spanish, and French", "identity", 0.95, "Trilingual. That's impressive."),
    ("I'm the CTO of a startup.", "User is the CTO of a startup", "work", 0.95, "Big role. What's the startup focused on?"),
    ("I run about 30 miles a week.", "User runs approximately 30 miles per week", "hobby", 0.85, "That's serious mileage. Impressive commitment."),
    ("My commute is about 45 minutes each way.", "User has a 45-minute commute each way", "general", 0.85, "90 minutes a day adds up. Podcast time?"),
    ("I'm gluten-free, not by choice. Celiac disease.", "User has celiac disease, must eat gluten-free", "health", 1.0, "Understood. I'll keep that in mind for any food suggestions."),
    ("I went to Stanford for my MBA.", "User has an MBA from Stanford", "identity", 0.95, "Strong program. That must have been intense."),
    ("My team has twelve people on it.", "User manages or is on a team of twelve people", "work", 0.85, "That's a decent-sized team."),
    ("I collect vinyl records.", "User collects vinyl records", "hobby", 0.9, "Nice hobby. What genres?"),
    ("I'm an only child.", "User is an only child", "family", 0.9, "That shapes a lot of how you see things."),
    ("I work the night shift, 10pm to 6am.", "User works night shift, 10pm to 6am", "work", 0.9, "Night shift. I'll adjust my timing for you."),
    ("I have anxiety issues, been dealing with it for years.", "User has dealt with anxiety for years", "health", 0.9, "That takes real strength. I'm here if you need me."),
    ("My favorite cuisine is Thai food.", "User's favorite cuisine is Thai food", "preference", 0.9, "Thai food is excellent. Hard to beat."),
    ("I'm based in London.", "User is based in London", "location", 0.95, "London's a great city to be in."),
    ("I do intermittent fasting, 16:8.", "User practices 16:8 intermittent fasting", "health", 0.85, "16:8 is popular. What's your eating window?"),
    ("I'm a freelancer, not employed by anyone.", "User is a freelancer, self-employed", "work", 0.9, "Freelancing gives you flexibility. How long have you been independent?"),
    ("My daughter's name is Sophie.", "User has a daughter named Sophie", "family", 0.95, "Sophie. Lovely name."),
    ("I've been sober for three years.", "User has been sober for three years", "health", 0.9, "Three years is a real achievement. Respect."),
    ("I use Arch Linux btw.", "User uses Arch Linux", "preference", 0.9, "Of course you do."),
    ("I prefer working in coffee shops over my home office.", "User prefers working in coffee shops over home office", "preference", 0.85, "The ambient noise probably helps focus."),
    ("I was born in December.", "User was born in December", "identity", 0.95, "A winter birthday."),
    ("My husband is a teacher.", "User's husband is a teacher", "family", 0.9, "Teachers do important work."),
    ("I'm color blind. Red-green specifically.", "User has red-green color blindness", "health", 0.95, "Good to know. I'll be mindful of that for any visual suggestions."),
    ("I do yoga every morning.", "User practices yoga every morning", "hobby", 0.85, "Great way to start the day."),
    ("My mortgage payment is about 2500 a month.", "User's mortgage payment is approximately $2500/month", "finance", 0.85, "That's pretty typical for most markets these days."),
    ("I'm on the keto diet.", "User follows the keto diet", "preference", 0.85, "Keto can be effective. How long have you been on it?"),
    ("I went to a coding bootcamp, didn't do a CS degree.", "User attended a coding bootcamp instead of a CS degree", "identity", 0.9, "Bootcamps can be incredibly effective. The practical focus is a strength."),
    ("I'm 42.", "User is 42 years old", "identity", 0.9, "The answer to everything."),
    ("My office is in downtown Seattle.", "User's office is in downtown Seattle", "location", 0.9, "Seattle's a good tech hub."),
    ("I have three kids.", "User has three children", "family", 0.9, "Three kids. You've got your hands full."),
    ("I meditate for 20 minutes every evening.", "User meditates for 20 minutes every evening", "hobby", 0.85, "That's a solid practice. Consistency is key."),
    ("I'm a backend developer mostly.", "User is primarily a backend developer", "work", 0.9, "Backend is where the interesting problems are."),
    ("My car lease is up next month.", "User's car lease expires next month", "finance", 0.8, "Time to decide. Lease again or buy?"),
    ("I've been married for fifteen years.", "User has been married for fifteen years", "family", 0.9, "Fifteen years is something to be proud of."),
    ("I play basketball on weekends.", "User plays basketball on weekends", "hobby", 0.85, "Good way to stay active and social."),
    ("I'm terrified of flying.", "User has a fear of flying", "preference", 0.9, "That makes travel tricky. Do you manage it somehow?"),
    ("My salary is around 180k.", "User's salary is approximately $180,000", "finance", 0.85, "Solid compensation."),
    ("I'm dyslexic.", "User is dyslexic", "health", 0.95, "Good to know. I'll keep that in mind for how I format things."),
    ("I homeschool my kids.", "User homeschools their children", "family", 0.9, "That's a big commitment. How's it going?"),
    ("I switched to a standing desk last year.", "User uses a standing desk, switched about a year ago", "preference", 0.85, "How's your back been since the switch?"),
    ("I'm in the Eastern time zone.", "User is in the Eastern time zone", "location", 0.9, "ET. Got it."),
    ("I have a PhD in physics.", "User has a PhD in physics", "identity", 0.95, "That's quite a background. How'd you end up where you are now?"),
    ("My mom has Alzheimer's.", "User's mother has Alzheimer's disease", "family", 0.9, "I'm sorry. That's incredibly hard."),
    ("I cycle to work every day.", "User cycles to work daily", "preference", 0.9, "Good for you and the environment. How far is the ride?"),
    ("I'm saving up for a house down payment.", "User is saving for a house down payment", "finance", 0.8, "Smart move. Any target timeline?"),
    ("I used to live in Tokyo for five years.", "User lived in Tokyo for five years", "identity", 0.9, "Five years in Tokyo. That must have been quite an experience."),
    ("I'm a night owl. Don't talk to me before noon.", "User is a night owl, prefers not to be disturbed before noon", "preference", 0.9, "Noted. I'll keep things quiet in the morning."),
    ("I coach my son's soccer team.", "User coaches their son's soccer team", "hobby", 0.85, "That's great father-son time."),
    ("I'm on a first-name basis with my CEO.", "User has a close working relationship with their CEO", "work", 0.8, "That's a good sign of company culture."),
    ("I drink about 4 cups of coffee a day.", "User drinks approximately 4 cups of coffee daily", "preference", 0.8, "That's some serious caffeine intake."),
    ("I have asthma.", "User has asthma", "health", 0.95, "Good to know. I'll keep that in mind."),
    ("I volunteer at a food bank on Saturdays.", "User volunteers at a food bank on Saturdays", "hobby", 0.85, "That's really admirable."),
    ("My rent is $3200 a month.", "User pays $3200/month in rent", "finance", 0.85, "That's up there. Must be a high-cost area."),
    ("I studied computer science at Berkeley.", "User studied computer science at UC Berkeley", "identity", 0.95, "Berkeley CS. Strong program."),
    ("I'm expecting twins.", "User is expecting twins", "family", 0.95, "Twins! Double the joy. And the chaos. Congratulations."),
    ("I take Sundays completely off. No screens.", "User takes Sundays off with no screens", "preference", 0.9, "Digital sabbath. Smart boundary."),
    ("I got my pilot's license last year.", "User obtained a pilot's license last year", "hobby", 0.9, "That's incredible. Do you fly often?"),
    ("I'm allergic to cats, unfortunately.", "User is allergic to cats", "health", 0.9, "That rules out a lot of internet culture."),
    ("My sister lives in Paris.", "User's sister lives in Paris", "family", 0.85, "Paris is a nice excuse to visit."),
    ("I invested most of my savings in index funds.", "User's savings are primarily in index funds", "finance", 0.8, "That's the Bogle approach. Hard to beat long term."),
    ("I'm a Sagittarius.", "User's zodiac sign is Sagittarius", "identity", 0.8, "Sagittarius. Adventurous type."),
    ("I wear glasses but I'm thinking about LASIK.", "User wears glasses and is considering LASIK surgery", "health", 0.8, "LASIK can be life-changing. Worth looking into."),
    ("My team just went fully remote.", "User's team recently transitioned to fully remote work", "work", 0.85, "That's the trend. How's the team adjusting?"),
    ("I have a cabin in Vermont that I visit every summer.", "User has a cabin in Vermont, visits every summer", "travel", 0.85, "Vermont in summer sounds ideal."),
    ("I don't eat red meat.", "User doesn't eat red meat", "preference", 0.9, "I'll keep that in mind."),
    ("I'm the oldest of four siblings.", "User is the oldest of four siblings", "family", 0.9, "Oldest child. That explains a lot, probably."),
    ("I make my own sourdough bread.", "User bakes their own sourdough bread", "hobby", 0.85, "Sourdough is an art. How's your starter doing?"),
    ("I just got a rescue dog. His name is Bear.", "User recently adopted a rescue dog named Bear", "family", 0.9, "Welcome home, Bear. Rescue dogs are the best."),
    ("I'm on my third startup.", "User is on their third startup", "work", 0.9, "Third time. You know the drill by now."),
    ("I moved to Canada from India when I was twelve.", "User moved from India to Canada at age twelve", "identity", 0.9, "That's a big move at that age. Must have been quite an adjustment."),
    ("My wife's name is Elena.", "User's wife's name is Elena", "family", 0.95, "Elena. I'll remember that."),
    ("I do cold plunges every morning.", "User does cold plunges every morning", "hobby", 0.85, "That takes discipline. How long have you been doing it?"),
    ("I'm pre-diabetic.", "User is pre-diabetic", "health", 0.95, "That's important to monitor. Are you working with a doctor on it?"),
    ("I live in a tiny apartment in Manhattan.", "User lives in a small apartment in Manhattan", "location", 0.9, "Manhattan living. Every square foot counts."),
    ("I read about 50 books a year.", "User reads approximately 50 books per year", "hobby", 0.85, "Almost one a week. That's impressive reading speed."),
    ("My annual bonus is coming up in April.", "User expects an annual bonus in April", "finance", 0.8, "Something to look forward to."),
    ("I manage a team of six engineers.", "User manages a team of six engineers", "work", 0.9, "Six is a manageable team size. Not too big, not too small."),
    ("I have a home gym in my garage.", "User has a home gym in their garage", "hobby", 0.85, "Home gym is the best investment. No commute, no waiting."),
    ("I use he/him pronouns.", "User uses he/him pronouns", "identity", 1.0, "Got it."),
    ("I'm Jewish.", "User is Jewish", "identity", 0.95, "Noted."),
    ("I'm severely nearsighted.", "User is severely nearsighted", "health", 0.9, "That's good context to have."),
    ("My favorite band is Radiohead.", "User's favorite band is Radiohead", "preference", 0.9, "Excellent taste."),
    ("I grow my own vegetables in the backyard.", "User grows vegetables in their backyard", "hobby", 0.85, "Nothing beats homegrown. What do you grow?"),
    ("I have a twin sister.", "User has a twin sister", "family", 0.9, "Twins. Are you identical or fraternal?"),
    ("I work four days a week.", "User works a four-day work week", "work", 0.9, "Four-day weeks are the future."),
    ("I'm saving about 30% of my income.", "User saves approximately 30% of their income", "finance", 0.85, "That's well above average. Good discipline."),
]


# ---------------------------------------------------------------------------
# CATEGORY 2: Correction-triggered remembering (80 examples)
# ---------------------------------------------------------------------------
CORRECTIONS = [
    # (user_msg, tool, args, response)
    # save_user_fact for corrected personal details
    ("No, I switched to Firefox last week.", "save_user_fact",
     {"fact": "User switched to Firefox browser", "domain": "preference", "confidence": 0.9},
     "Firefox it is. Updated."),
    ("Actually my meeting is at 3, not 2.", "remember",
     {"text": "User's meeting time corrected to 3pm", "importance": 0.8, "memory_type": "episodic", "domain": "work"},
     "3pm. Got it."),
    ("I told you, I don't like coffee.", "save_user_fact",
     {"fact": "User does not like coffee", "domain": "preference", "confidence": 0.95},
     "Right, sorry about that. I've noted it down."),
    ("That's not my manager, Sarah is my manager now.", "save_user_fact",
     {"fact": "User's current manager is Sarah", "domain": "work", "confidence": 0.9},
     "Sarah. Updated."),
    ("No, I said I have three kids, not two.", "save_user_fact",
     {"fact": "User has three children", "domain": "family", "confidence": 0.95},
     "Three kids. Corrected, sorry about that."),
    ("I don't work at Microsoft anymore. I'm at Amazon now.", "save_user_fact",
     {"fact": "User now works at Amazon, previously worked at Microsoft", "domain": "work", "confidence": 0.95},
     "Amazon now. Got it, my mistake."),
    ("Actually I moved. I'm in Chicago now, not Austin.", "save_user_fact",
     {"fact": "User moved to Chicago from Austin", "domain": "location", "confidence": 0.95},
     "Chicago. Updated."),
    ("It's not my girlfriend, she's my fiancee now. We got engaged.", "save_user_fact",
     {"fact": "User is now engaged, partner is fiancee", "domain": "family", "confidence": 0.95},
     "Congratulations on the engagement! Updated."),
    ("I actually don't use Vim anymore. Switched to Neovim.", "save_user_fact",
     {"fact": "User switched from Vim to Neovim", "domain": "preference", "confidence": 0.9},
     "Neovim. Modern choice."),
    ("No, my birthday is the 22nd, not the 20th.", "save_user_fact",
     {"fact": "User's birthday is on the 22nd", "domain": "identity", "confidence": 1.0},
     "The 22nd. Fixed."),
    ("I quit running, it was hurting my knees. I swim now.", "save_user_fact",
     {"fact": "User quit running due to knee issues, now swims instead", "domain": "hobby", "confidence": 0.9},
     "Swimming is much easier on the joints. Good switch."),
    ("Stop suggesting Indian food, I told you I'm not into spicy food.", "save_user_fact",
     {"fact": "User does not like spicy food, avoid suggesting Indian food", "domain": "preference", "confidence": 0.95},
     "Apologies. No spicy food. Won't suggest it again."),
    ("I'm not a junior anymore, I got promoted to mid-level last month.", "save_user_fact",
     {"fact": "User was promoted to mid-level engineer last month", "domain": "work", "confidence": 0.9},
     "Mid-level now. Congrats on the promotion."),
    ("Actually we sold the Tesla. Got a Rivian instead.", "save_user_fact",
     {"fact": "User sold their Tesla and bought a Rivian", "domain": "general", "confidence": 0.9},
     "Rivian. Nice upgrade."),
    ("I changed my mind about the keto diet. Back to normal eating.", "save_user_fact",
     {"fact": "User stopped the keto diet, returned to regular eating", "domain": "preference", "confidence": 0.9},
     "Back to regular eating. Noted."),
    ("It's Dr. Patel actually, not Ms. Patel. I have a PhD.", "save_user_fact",
     {"fact": "User has a PhD, prefers to be addressed as Dr. Patel", "domain": "identity", "confidence": 1.0},
     "Dr. Patel. Apologies for the mix-up."),
    ("I don't live alone anymore. My partner moved in.", "save_user_fact",
     {"fact": "User's partner moved in, no longer lives alone", "domain": "family", "confidence": 0.9},
     "That's a big step. Hope it's going well."),
    ("My schedule changed. I'm doing 4 days a week now, not 5.", "save_user_fact",
     {"fact": "User now works 4 days per week, changed from 5", "domain": "work", "confidence": 0.9},
     "Four-day week. That's an improvement."),
    ("No, I use macOS now, I switched from Linux.", "save_user_fact",
     {"fact": "User switched from Linux to macOS", "domain": "preference", "confidence": 0.9},
     "macOS. Updated."),
    ("I'm not freelancing anymore, I took a full-time position.", "save_user_fact",
     {"fact": "User transitioned from freelancing to a full-time position", "domain": "work", "confidence": 0.9},
     "Full-time now. How's the transition been?"),
    ("Actually it's a different project now. We pivoted to a B2B model.", "remember",
     {"text": "User's company pivoted to a B2B model", "importance": 0.8, "memory_type": "episodic", "domain": "work"},
     "B2B pivot. That changes a lot. How's it shaping up?"),
    ("Wait, I need to correct myself. The meeting is Thursday, not Wednesday.", "remember",
     {"text": "User corrected meeting day to Thursday, not Wednesday", "importance": 0.7, "memory_type": "episodic", "domain": "work"},
     "Thursday. Thanks for the correction."),
    ("I stopped meditating actually. Couldn't keep up with it.", "save_user_fact",
     {"fact": "User stopped meditating", "domain": "hobby", "confidence": 0.85},
     "It happens. Maybe you'll pick it back up someday."),
    ("Don't call me Alex. I go by Sasha now.", "save_user_fact",
     {"fact": "User prefers to be called Sasha, not Alex", "domain": "identity", "confidence": 1.0},
     "Sasha. Got it. Sorry about that."),
    ("I didn't get the promotion actually. They gave it to someone else.", "remember",
     {"text": "User did not get the expected promotion, it went to someone else", "importance": 0.7, "memory_type": "episodic", "domain": "work"},
     "I'm sorry to hear that. That's frustrating."),
    ("No, we broke up. I'm single now.", "save_user_fact",
     {"fact": "User is now single after a breakup", "domain": "family", "confidence": 0.9},
     "I'm sorry. That's tough."),
    ("I actually switched to decaf. Trying to cut down on caffeine.", "save_user_fact",
     {"fact": "User switched to decaf coffee, reducing caffeine intake", "domain": "preference", "confidence": 0.85},
     "Decaf. Good for sleep probably."),
    ("My new address is in Brooklyn, not Queens.", "save_user_fact",
     {"fact": "User lives in Brooklyn, not Queens", "domain": "location", "confidence": 0.95},
     "Brooklyn. Updated."),
    ("I use they/them pronouns actually.", "save_user_fact",
     {"fact": "User uses they/them pronouns", "domain": "identity", "confidence": 1.0},
     "They/them. Noted. Thanks for letting me know."),
    ("No, I changed doctors. I see Dr. Kim now.", "save_user_fact",
     {"fact": "User's current doctor is Dr. Kim", "domain": "health", "confidence": 0.9},
     "Dr. Kim. Updated."),
    ("I switched from Python to Go for my main work.", "save_user_fact",
     {"fact": "User's primary programming language is now Go, switched from Python", "domain": "work", "confidence": 0.9},
     "Go. It's a good fit for a lot of backend work."),
    ("Actually I'm 35, not 34. My birthday was last week.", "save_user_fact",
     {"fact": "User is 35 years old, birthday was recently", "domain": "identity", "confidence": 0.95},
     "Happy belated birthday. 35 it is."),
    ("I don't eat sushi anymore. Had a bad experience.", "save_user_fact",
     {"fact": "User no longer eats sushi", "domain": "preference", "confidence": 0.9},
     "No sushi. Understood."),
    ("I started taking antidepressants last month.", "save_user_fact",
     {"fact": "User started antidepressant medication last month", "domain": "health", "confidence": 0.9},
     "That takes courage. Hope it helps."),
    ("I'm not on the infra team anymore. I moved to the platform team.", "save_user_fact",
     {"fact": "User moved from infrastructure team to platform team", "domain": "work", "confidence": 0.9},
     "Platform team. How's the new team?"),
    ("We changed the deployment to Tuesdays now.", "remember",
     {"text": "Team changed deployment day to Tuesdays", "importance": 0.7, "memory_type": "episodic", "domain": "work"},
     "Tuesdays now. Got it."),
    ("I stopped intermittent fasting. It wasn't working for me.", "save_user_fact",
     {"fact": "User stopped intermittent fasting", "domain": "health", "confidence": 0.85},
     "Not everything works for everyone. Fair enough."),
    ("I got a new cat. Her name is Pepper, not the old name I told you.", "save_user_fact",
     {"fact": "User got a new cat named Pepper", "domain": "family", "confidence": 0.9},
     "Pepper. Welcome to the family."),
    ("I actually use VS Code now, not IntelliJ.", "save_user_fact",
     {"fact": "User switched from IntelliJ to VS Code", "domain": "preference", "confidence": 0.9},
     "VS Code. It's come a long way."),
    ("I dropped out of the marathon. Knee injury.", "remember",
     {"text": "User dropped out of marathon training due to knee injury", "importance": 0.6, "memory_type": "episodic", "domain": "health"},
     "Sorry about the knee. Take care of it."),
    ("I'm not in the Eastern time zone anymore. I moved to Pacific.", "save_user_fact",
     {"fact": "User is now in the Pacific time zone, moved from Eastern", "domain": "location", "confidence": 0.95},
     "Pacific time. Updated."),
    ("No, I sold the house. We're renting now.", "save_user_fact",
     {"fact": "User sold their house and is now renting", "domain": "location", "confidence": 0.9},
     "Renting now. Hope the sale went well."),
    ("I changed my diet. I'm pescatarian now.", "save_user_fact",
     {"fact": "User is now pescatarian", "domain": "preference", "confidence": 0.9},
     "Pescatarian. Fish but no other meat. Got it."),
    ("No, I left Google. I'm at a startup now.", "save_user_fact",
     {"fact": "User left Google and joined a startup", "domain": "work", "confidence": 0.95},
     "Big change from Google to startup life. How's it going?"),
    ("Actually my daughter is in first grade now, not kindergarten.", "save_user_fact",
     {"fact": "User's daughter is in first grade", "domain": "family", "confidence": 0.9},
     "First grade already. Time flies. Updated."),
    ("I stopped using Spotify. Switched to Apple Music.", "save_user_fact",
     {"fact": "User switched from Spotify to Apple Music", "domain": "preference", "confidence": 0.9},
     "Apple Music. Noted."),
    ("I'm not on medication anymore. Doctor took me off it.", "save_user_fact",
     {"fact": "User is no longer on medication, discontinued by doctor", "domain": "health", "confidence": 0.9},
     "That's good news if the doctor says so."),
    ("I changed my morning routine. I do a workout first thing now.", "save_user_fact",
     {"fact": "User's morning routine now starts with a workout", "domain": "preference", "confidence": 0.85},
     "Morning workouts. Good shift."),
    ("I'm lead now, not just senior. Got promoted last week.", "save_user_fact",
     {"fact": "User was promoted to lead from senior level last week", "domain": "work", "confidence": 0.9},
     "Lead now. Congrats!"),
    ("I gave up coffee entirely. Herbal tea only.", "save_user_fact",
     {"fact": "User quit coffee entirely, drinks only herbal tea", "domain": "preference", "confidence": 0.9},
     "Herbal tea only. That's a commitment."),
    ("Actually we're not expecting twins. The second scan showed just one.", "save_user_fact",
     {"fact": "User is expecting one baby, not twins as previously thought", "domain": "family", "confidence": 0.95},
     "One baby. Still wonderful news."),
    ("I retired from coaching. Too much time commitment.", "save_user_fact",
     {"fact": "User stopped coaching soccer team due to time constraints", "domain": "hobby", "confidence": 0.85},
     "Understandable. You've got a lot on your plate."),
    ("Correct me if you have it wrong, but I'm 28, not 30.", "save_user_fact",
     {"fact": "User is 28 years old", "domain": "identity", "confidence": 0.95},
     "28. Fixed."),
    ("We moved the standup to 9:30, not 9.", "remember",
     {"text": "Team standup meeting moved to 9:30 AM from 9 AM", "importance": 0.6, "memory_type": "episodic", "domain": "work"},
     "9:30 standup. Got it."),
]


# ---------------------------------------------------------------------------
# CATEGORY 3: Event and experience storage (80 examples)
# ---------------------------------------------------------------------------
EVENTS = [
    # (user_msg, text, importance, memory_type, domain, response)
    ("The interview went really well today.", "User had a successful interview today", 0.7, "episodic", "work", "That's great. Fingers crossed for the next steps."),
    ("I had a terrible migraine all afternoon.", "User had a severe migraine this afternoon", 0.6, "episodic", "health", "That sounds rough. Feeling better now?"),
    ("We just closed the Series B!", "User's company closed Series B funding", 0.9, "episodic", "work", "That's huge. Congratulations to the whole team."),
    ("My car broke down on the highway.", "User's car broke down on the highway", 0.7, "episodic", "general", "That's stressful. Are you safe? Did you get help?"),
    ("I got into a fender bender this morning.", "User was in a minor car accident this morning", 0.7, "episodic", "general", "Are you okay? Hope nobody was hurt."),
    ("My boss just told me the project is cancelled.", "User's project was cancelled by management", 0.8, "episodic", "work", "That's frustrating after all the work you put in."),
    ("I passed the certification exam!", "User passed a certification exam", 0.8, "episodic", "work", "Congrats! All that studying paid off."),
    ("The server went down for two hours today.", "Production server had a two-hour outage today", 0.7, "episodic", "work", "Two hours is a significant outage. What happened?"),
    ("My kid said her first word today!", "User's child said their first word today", 0.9, "episodic", "family", "That's a moment you'll never forget. What was the word?"),
    ("I just ran my first 10K.", "User completed their first 10K race", 0.7, "episodic", "hobby", "That's an accomplishment. How'd you feel?"),
    ("We had layoffs at work today. I survived but it's terrible.", "User's company had layoffs, user was not affected but is upset", 0.8, "episodic", "work", "That's awful even when you're not directly hit. How are you doing?"),
    ("I finally finished writing my book.", "User finished writing a book", 0.8, "episodic", "hobby", "That's a massive achievement. What's it about?"),
    ("My dad had a heart attack. He's in the hospital.", "User's father had a heart attack and is hospitalized", 0.9, "episodic", "family", "I'm so sorry. Is he stable? Please take care of yourself too."),
    ("I got the job offer! They want me to start in two weeks.", "User received a job offer, start date in two weeks", 0.9, "episodic", "work", "That's fantastic news. Congratulations."),
    ("Power has been out for three hours.", "Power outage at user's location for three hours", 0.5, "episodic", "general", "That's annoying. Any estimate on when it'll be back?"),
    ("I submitted my grad school application today.", "User submitted graduate school application", 0.7, "episodic", "work", "Exciting step. Which program?"),
    ("We shipped the product! It's live.", "User's product launched", 0.9, "episodic", "work", "Live! That must feel incredible after all the work."),
    ("I twisted my ankle playing basketball.", "User twisted their ankle playing basketball", 0.6, "episodic", "health", "Ouch. Ice and elevation. Take it easy."),
    ("The client loved the presentation.", "User's client presentation was well received", 0.7, "episodic", "work", "That's a win. All the prep paid off."),
    ("We adopted a dog from the shelter today.", "User adopted a dog from a shelter", 0.8, "episodic", "family", "That's wonderful. What kind?"),
    ("I got a raise! 15% increase.", "User received a 15% salary raise", 0.8, "episodic", "work", "Well deserved. That's a solid bump."),
    ("My flight got cancelled. Stuck at the airport.", "User's flight was cancelled, stranded at airport", 0.6, "episodic", "travel", "That's the worst. Any rebooking options?"),
    ("I won the hackathon!", "User won a hackathon", 0.8, "episodic", "work", "Amazing! What did you build?"),
    ("My grandmother passed away last night.", "User's grandmother passed away", 0.9, "episodic", "family", "I'm deeply sorry for your loss."),
    ("The code review went terribly. They want a full rewrite.", "User's code review went poorly, rewrite requested", 0.6, "episodic", "work", "Painful but sometimes that's what's needed. What were the main issues?"),
    ("I made dinner for 12 people tonight and it was actually good.", "User successfully cooked dinner for 12 people", 0.5, "episodic", "general", "Cooking for 12 is no joke. What did you make?"),
    ("I lost my wallet today.", "User lost their wallet", 0.7, "episodic", "general", "That's stressful. Have you cancelled your cards?"),
    ("We signed the lease on the new office.", "User's company signed a lease on a new office", 0.7, "episodic", "work", "New office. That's a sign of growth."),
    ("I had my first therapy session today.", "User had their first therapy session", 0.7, "episodic", "health", "That takes courage. How did it go?"),
    ("The demo crashed in front of the investors.", "User's product demo crashed during investor presentation", 0.8, "episodic", "work", "Nightmare scenario. How did they react?"),
    ("I found out I got accepted into the program!", "User was accepted into a program they applied to", 0.8, "episodic", "work", "Congratulations! All that effort paid off."),
    ("My tooth cracked. Going to the dentist tomorrow.", "User has a cracked tooth, dental appointment tomorrow", 0.6, "episodic", "health", "That's uncomfortable. Hope it's a quick fix."),
    ("I gave my first conference talk today.", "User delivered their first conference presentation", 0.8, "episodic", "work", "That's a milestone. How did the audience respond?"),
    ("We just hit 1000 users.", "User's product reached 1000 users", 0.8, "episodic", "work", "1000 users is a real milestone. Onwards."),
    ("My best friend just got diagnosed with cancer.", "User's best friend was diagnosed with cancer", 0.8, "episodic", "family", "I'm really sorry. That's devastating news. Be there for them."),
    ("I deleted the production database by accident.", "User accidentally deleted production database", 0.8, "episodic", "work", "That's every engineer's worst nightmare. Do you have backups?"),
    ("My partner proposed last night!", "User's partner proposed marriage", 0.9, "episodic", "family", "Congratulations! How did it happen?"),
    ("I finally paid off my student loans.", "User paid off student loans in full", 0.8, "episodic", "finance", "That's a huge weight lifted. Congratulations."),
    ("The house inspection found major foundation issues.", "User's house inspection revealed foundation problems", 0.7, "episodic", "general", "That's concerning. What are the options?"),
    ("I got my first open source PR merged.", "User's first open source pull request was merged", 0.7, "episodic", "work", "First one's always special. What project?"),
    ("We lost the big client. They went with a competitor.", "User's company lost a major client to a competitor", 0.8, "episodic", "work", "That stings. Any lessons to take from it?"),
    ("I just found out I'm pregnant.", "User found out they are pregnant", 0.9, "episodic", "family", "Incredible news. Congratulations."),
    ("My apartment got broken into.", "User's apartment was burglarized", 0.8, "episodic", "general", "That's violation. Are you safe? Did you file a report?"),
    ("I finished the Ironman triathlon.", "User completed an Ironman triathlon", 0.8, "episodic", "hobby", "Ironman. That's elite territory. Incredible achievement."),
    ("Our startup just got acquired.", "User's startup was acquired", 0.9, "episodic", "work", "That's a massive milestone. How do you feel about it?"),
    ("I had a panic attack at work today.", "User had a panic attack at work", 0.7, "episodic", "health", "That sounds really tough. Are you feeling safe now?"),
    ("The doctor said the test results are all clear.", "User's medical test results came back clear", 0.7, "episodic", "health", "That's a relief. Great news."),
    ("I got my citizenship approved!", "User's citizenship application was approved", 0.9, "episodic", "identity", "That's a life milestone. Congratulations."),
    ("The roof is leaking. Water everywhere.", "User's roof is leaking causing water damage", 0.7, "episodic", "general", "That needs immediate attention. Have you called someone?"),
    ("I met someone new. We went on our first date.", "User went on a first date with someone new", 0.6, "episodic", "family", "How did it go?"),
    ("My dog had surgery today. Everything went well.", "User's dog had successful surgery", 0.7, "episodic", "family", "Glad to hear the surgery went well. Give them extra treats during recovery."),
    ("I just got back from two weeks in Japan. It was incredible.", "User returned from a two-week trip to Japan", 0.6, "episodic", "travel", "Japan is something else. What was the highlight?"),
    ("I failed the driving test. Second time.", "User failed driving test for the second time", 0.6, "episodic", "general", "Frustrating. But most people pass on the third. You'll get it."),
    ("My team won the company hackathon.", "User's team won company hackathon", 0.7, "episodic", "work", "Nice. Team effort. What did you build?"),
    ("I just had the most productive week I've had in months.", "User had an exceptionally productive work week", 0.5, "episodic", "work", "Those weeks feel great. What clicked?"),
    ("The pipe burst in the basement. Flooded everything.", "User's basement flooded due to burst pipe", 0.7, "episodic", "general", "What a mess. Insurance should cover most of that."),
    ("I got my first consulting client.", "User secured their first consulting client", 0.7, "episodic", "work", "First client. The hardest one to get. Congratulations."),
    ("My mom just turned 70. We threw her a surprise party.", "User's mother turned 70, surprise party thrown", 0.6, "episodic", "family", "Happy birthday to her! I bet she was surprised."),
    ("I crashed my bike today. Scraped up pretty bad.", "User crashed their bicycle and was injured", 0.6, "episodic", "health", "Ouch. Nothing broken? Take care of those scrapes."),
    ("I just hit 100 consecutive days of working out.", "User reached 100 consecutive workout days", 0.6, "episodic", "hobby", "100 days is serious consistency. That habit is locked in."),
    ("My paper got accepted at the conference!", "User's research paper was accepted at a conference", 0.8, "episodic", "work", "Accepted! Which conference?"),
    ("I had to fire someone today. It was awful.", "User had to terminate an employee", 0.7, "episodic", "work", "That's one of the hardest things in management. Are you okay?"),
    ("We moved into the new house this weekend.", "User moved into their new house", 0.7, "episodic", "general", "New house! How's the settling in going?"),
    ("I just booked a trip to Iceland for next month.", "User booked a trip to Iceland for next month", 0.6, "episodic", "travel", "Iceland is stunning. You're going to love it."),
    ("My kid won the spelling bee!", "User's child won a spelling bee", 0.6, "episodic", "family", "That's wonderful! They must be thrilled."),
    ("The investor meeting went sideways. They passed.", "User's investor pitch was rejected", 0.7, "episodic", "work", "Rough. But it only takes one yes. What was their feedback?"),
    ("I got locked out of my apartment in the rain.", "User was locked out of apartment in the rain", 0.4, "episodic", "general", "Classic bad day scenario. Did you get back in?"),
    ("I won $500 in a poker tournament.", "User won $500 in a poker tournament", 0.5, "episodic", "hobby", "Nice pot. Do you play often?"),
    ("My kid took her first steps today!", "User's child took their first steps", 0.9, "episodic", "family", "Those first steps. What a moment. Treasure it."),
    ("We hit our quarterly revenue target.", "User's company met quarterly revenue target", 0.7, "episodic", "work", "Target hit. Good execution by the team."),
    ("I got food poisoning from that new restaurant.", "User got food poisoning from a new restaurant", 0.5, "episodic", "health", "That's miserable. Stay hydrated. Give it a day."),
    ("The build is broken and nobody knows why.", "Production build is broken with no clear cause", 0.6, "episodic", "work", "Mystery build failures are the worst. Have you checked recent commits?"),
    ("I completed my first triathlon.", "User completed their first triathlon", 0.7, "episodic", "hobby", "That's a huge accomplishment. What were your splits?"),
    ("We celebrated our 10th anniversary last night.", "User celebrated 10th wedding anniversary", 0.7, "episodic", "family", "Ten years. That's something special. How did you celebrate?"),
    ("I got into a car accident. I'm okay but the car is totaled.", "User was in a car accident, uninjured but car totaled", 0.8, "episodic", "general", "Thank god you're okay. The car can be replaced."),
    ("I had my wisdom teeth out today.", "User had wisdom teeth extracted", 0.5, "episodic", "health", "Rest up. The next couple days will be rough but it gets better fast."),
    ("My startup just crossed $1M ARR.", "User's startup reached $1 million in annual recurring revenue", 0.9, "episodic", "work", "Million dollar run rate. That's a serious milestone."),
    ("I've been having trouble sleeping for the past week.", "User experiencing sleep difficulties for the past week", 0.6, "episodic", "health", "A week of bad sleep adds up fast. Any idea what's causing it?"),
    ("I just voted for the first time.", "User voted for the first time", 0.7, "episodic", "general", "That's an important first. How did it feel?"),
]


# ---------------------------------------------------------------------------
# CATEGORY 4: Preferences and opinions (60 examples)
# ---------------------------------------------------------------------------
PREFERENCES = [
    # (user_msg, tool, args, response)
    ("I really prefer dark mode for everything.", "save_user_fact",
     {"fact": "User prefers dark mode for all applications", "domain": "preference", "confidence": 0.9},
     "Dark mode everywhere. Agreed."),
    ("TypeScript is way better than JavaScript imo.", "remember",
     {"text": "User prefers TypeScript over JavaScript", "importance": 0.5, "memory_type": "semantic", "domain": "preference"},
     "The type safety alone makes a strong case."),
    ("I can't stand meetings before 10am.", "save_user_fact",
     {"fact": "User dislikes meetings before 10am", "domain": "preference", "confidence": 0.85},
     "No early meetings. Understood."),
    ("I always take the train, never drive to work.", "save_user_fact",
     {"fact": "User commutes by train, doesn't drive to work", "domain": "preference", "confidence": 0.9},
     "Train commuter. Good for reading time."),
    ("I think Go is overrated. Lack of generics was a dealbreaker.", "remember",
     {"text": "User thinks Go is overrated, dislikes the lack of generics", "importance": 0.4, "memory_type": "semantic", "domain": "preference"},
     "The generics situation has improved but yeah, it was rough for a long time."),
    ("I prefer small focused teams over big ones.", "save_user_fact",
     {"fact": "User prefers small focused teams over large teams", "domain": "preference", "confidence": 0.85},
     "Smaller teams move faster. Makes sense."),
    ("I like working in silence, no music.", "save_user_fact",
     {"fact": "User prefers working in silence, no background music", "domain": "preference", "confidence": 0.9},
     "Pure focus. Respected."),
    ("I'm a tab person, not spaces.", "save_user_fact",
     {"fact": "User prefers tabs over spaces for indentation", "domain": "preference", "confidence": 0.9},
     "The eternal debate. Tabs it is."),
    ("I despise Jira with a passion.", "remember",
     {"text": "User strongly dislikes Jira", "importance": 0.5, "memory_type": "semantic", "domain": "preference"},
     "You're not alone on that one."),
    ("I'd rather use the terminal than any GUI.", "save_user_fact",
     {"fact": "User prefers command-line terminal over GUI applications", "domain": "preference", "confidence": 0.9},
     "Terminal first. A person of taste."),
    ("I like to plan everything in advance. Spontaneity stresses me out.", "save_user_fact",
     {"fact": "User prefers planning ahead, dislikes spontaneity", "domain": "preference", "confidence": 0.85},
     "Noted. I'll give you plenty of advance notice for things."),
    ("I always read documentation before trying something new.", "save_user_fact",
     {"fact": "User always reads documentation before trying new things", "domain": "preference", "confidence": 0.85},
     "RTFM first. Disciplined approach."),
    ("I find microservices way over-engineered for most use cases.", "remember",
     {"text": "User thinks microservices are over-engineered for most use cases", "importance": 0.4, "memory_type": "semantic", "domain": "preference"},
     "Monoliths have made a comeback for good reason."),
    ("I hate being interrupted when I'm in flow state.", "save_user_fact",
     {"fact": "User strongly dislikes being interrupted during flow state", "domain": "preference", "confidence": 0.9},
     "I'll be mindful of that. No unnecessary interruptions."),
    ("I prefer async communication over meetings.", "save_user_fact",
     {"fact": "User prefers async communication over synchronous meetings", "domain": "preference", "confidence": 0.9},
     "Async-first. Good policy."),
    ("I like mechanical keyboards with cherry brown switches.", "save_user_fact",
     {"fact": "User likes mechanical keyboards with Cherry Brown switches", "domain": "preference", "confidence": 0.9},
     "Cherry Browns. Good balance of tactile and not-too-loud."),
    ("I think React is past its prime. Svelte is the future.", "remember",
     {"text": "User thinks React is declining and Svelte is the future", "importance": 0.4, "memory_type": "semantic", "domain": "preference"},
     "Svelte's developer experience is hard to beat."),
    ("I can't stand cold weather. Anything below 60F and I'm miserable.", "save_user_fact",
     {"fact": "User dislikes cold weather, uncomfortable below 60F", "domain": "preference", "confidence": 0.85},
     "Warm weather person. Noted."),
    ("I always take the stairs. Never the elevator.", "save_user_fact",
     {"fact": "User always takes stairs instead of elevator", "domain": "preference", "confidence": 0.85},
     "Good habit. Built-in exercise."),
    ("I think pair programming is incredibly effective.", "remember",
     {"text": "User is a strong advocate of pair programming", "importance": 0.5, "memory_type": "semantic", "domain": "preference"},
     "The research backs that up for complex problems."),
    ("I refuse to use Chrome. Privacy nightmare.", "save_user_fact",
     {"fact": "User refuses to use Chrome due to privacy concerns", "domain": "preference", "confidence": 0.9},
     "Fair point. What browser do you use instead?"),
    ("I love working late at night when it's quiet.", "save_user_fact",
     {"fact": "User enjoys working late at night when it's quiet", "domain": "preference", "confidence": 0.85},
     "Late night focus sessions. There's something about the quiet."),
    ("I'm not a fan of social media. Deleted everything last year.", "save_user_fact",
     {"fact": "User deleted all social media accounts, not a fan of social media", "domain": "preference", "confidence": 0.9},
     "Digital minimalism. Hard choice but probably healthy."),
    ("I prefer physical books over ebooks.", "save_user_fact",
     {"fact": "User prefers physical books over ebooks", "domain": "preference", "confidence": 0.85},
     "Nothing beats the feel of a real book."),
    ("Agile is fine in theory but terrible in practice at most companies.", "remember",
     {"text": "User thinks Agile methodology is poorly implemented at most companies", "importance": 0.4, "memory_type": "semantic", "domain": "preference"},
     "The gap between Agile theory and practice is massive. Agreed."),
    ("I strongly prefer functional programming over OOP.", "save_user_fact",
     {"fact": "User prefers functional programming over object-oriented programming", "domain": "preference", "confidence": 0.85},
     "FP makes reasoning about code much cleaner."),
    ("I do all my writing in Markdown.", "save_user_fact",
     {"fact": "User writes everything in Markdown", "domain": "preference", "confidence": 0.9},
     "Markdown is universal. Good choice."),
    ("I think TDD slows me down more than it helps.", "remember",
     {"text": "User finds TDD slows them down rather than helping", "importance": 0.4, "memory_type": "semantic", "domain": "preference"},
     "It's a tradeoff. Works well for some, not for others."),
    ("I need at least 8 hours of sleep or I'm useless.", "save_user_fact",
     {"fact": "User needs at least 8 hours of sleep to function well", "domain": "preference", "confidence": 0.85},
     "Sleep is non-negotiable. I'll keep that in mind."),
    ("I prefer one-on-ones over group meetings.", "save_user_fact",
     {"fact": "User prefers one-on-one meetings over group meetings", "domain": "preference", "confidence": 0.85},
     "One-on-ones are more productive. Good preference."),
    ("I hate notifications. I keep everything on silent.", "save_user_fact",
     {"fact": "User keeps all notifications silenced, dislikes notifications", "domain": "preference", "confidence": 0.9},
     "I'll be very judicious about any alerts then."),
    ("I think Postgres is the best database. Period.", "remember",
     {"text": "User considers PostgreSQL the best database", "importance": 0.5, "memory_type": "semantic", "domain": "preference"},
     "Hard to argue with Postgres. It does everything."),
    ("I'm a big fan of the Pomodoro technique.", "save_user_fact",
     {"fact": "User uses the Pomodoro technique for time management", "domain": "preference", "confidence": 0.85},
     "25 on, 5 off. It works."),
    ("I really don't like video calls. Voice only is fine.", "save_user_fact",
     {"fact": "User prefers voice-only calls over video calls", "domain": "preference", "confidence": 0.85},
     "Camera off. I get it. Less draining."),
    ("I always use dual monitors. Can't work on a single screen.", "save_user_fact",
     {"fact": "User requires dual monitors, can't work on single screen", "domain": "preference", "confidence": 0.9},
     "Dual monitors are hard to go back from."),
    ("I think Kubernetes is way too complex for most teams.", "remember",
     {"text": "User thinks Kubernetes is overly complex for most teams", "importance": 0.4, "memory_type": "semantic", "domain": "preference"},
     "The operational overhead is real. Not every team needs it."),
    ("I prefer Linux over macOS and Windows.", "save_user_fact",
     {"fact": "User prefers Linux over macOS and Windows", "domain": "preference", "confidence": 0.9},
     "Linux person. Which distro?"),
    ("I'm a morning workout person. Evening workouts don't stick.", "save_user_fact",
     {"fact": "User prefers morning workouts, evening workouts don't work for them", "domain": "preference", "confidence": 0.85},
     "Morning workouts set the tone for the day."),
    ("I think open plan offices are the worst invention ever.", "remember",
     {"text": "User strongly dislikes open plan offices", "importance": 0.5, "memory_type": "semantic", "domain": "preference"},
     "The research agrees. Productivity killer."),
    ("I prefer short emails. Two sentences max.", "save_user_fact",
     {"fact": "User prefers short emails, two sentences maximum", "domain": "preference", "confidence": 0.85},
     "Brevity is a virtue."),
    ("I think static typing prevents so many bugs.", "remember",
     {"text": "User is a strong advocate of static typing", "importance": 0.4, "memory_type": "semantic", "domain": "preference"},
     "Catching bugs at compile time beats runtime surprises."),
    ("I never eat at my desk. Always take a proper lunch break.", "save_user_fact",
     {"fact": "User always takes a proper lunch break, doesn't eat at desk", "domain": "preference", "confidence": 0.85},
     "That's a healthy boundary most people skip."),
    ("I think Docker is essential for modern development.", "remember",
     {"text": "User considers Docker essential for modern development", "importance": 0.4, "memory_type": "semantic", "domain": "preference"},
     "Consistent environments everywhere. Hard to disagree."),
    ("I hate driving in traffic. I'll take the long way to avoid it.", "save_user_fact",
     {"fact": "User hates traffic, prefers longer routes to avoid it", "domain": "preference", "confidence": 0.85},
     "Moving beats sitting. Makes sense."),
    ("I strongly prefer written specs before coding.", "save_user_fact",
     {"fact": "User prefers written specifications before starting to code", "domain": "preference", "confidence": 0.9},
     "Think first, code second. Solid approach."),
    ("I find standup meetings completely useless.", "remember",
     {"text": "User finds standup meetings useless", "importance": 0.5, "memory_type": "semantic", "domain": "preference"},
     "A lot of them are just status reports that could be a message."),
    ("I'm obsessed with keyboard shortcuts. Mouse is too slow.", "save_user_fact",
     {"fact": "User prefers keyboard shortcuts over mouse interaction", "domain": "preference", "confidence": 0.9},
     "Keyboard-first workflow. Efficient."),
    ("I think Rust is the best systems language that exists.", "remember",
     {"text": "User considers Rust the best systems programming language", "importance": 0.5, "memory_type": "semantic", "domain": "preference"},
     "The ownership model is genius. Hard to argue."),
    ("I prefer cooking at home over eating out.", "save_user_fact",
     {"fact": "User prefers cooking at home over dining out", "domain": "preference", "confidence": 0.85},
     "Healthier and cheaper. Good call."),
    ("I think code comments are usually a sign of unclear code.", "remember",
     {"text": "User believes code comments indicate unclear code, prefers self-documenting code", "importance": 0.4, "memory_type": "semantic", "domain": "preference"},
     "Self-documenting code is the goal. Comments for 'why', not 'what'."),
    ("I like keeping my inbox at zero. Unread emails stress me out.", "save_user_fact",
     {"fact": "User practices inbox zero, unread emails cause stress", "domain": "preference", "confidence": 0.85},
     "Inbox zero. Takes discipline but worth it."),
    ("I find Slack more distracting than useful.", "remember",
     {"text": "User finds Slack more distracting than useful", "importance": 0.5, "memory_type": "semantic", "domain": "preference"},
     "The constant notifications can kill productivity."),
    ("I love walking meetings. Way better than sitting in a room.", "save_user_fact",
     {"fact": "User prefers walking meetings over traditional sit-down meetings", "domain": "preference", "confidence": 0.85},
     "Walking meetings boost creativity. Good approach."),
    ("I prefer self-hosted solutions over cloud SaaS.", "save_user_fact",
     {"fact": "User prefers self-hosted solutions over cloud SaaS", "domain": "preference", "confidence": 0.85},
     "Self-hosted gives you control. Worth the setup."),
    ("I think AI-generated code needs heavy review. Never trust it blindly.", "remember",
     {"text": "User believes AI-generated code requires heavy review and shouldn't be trusted blindly", "importance": 0.5, "memory_type": "semantic", "domain": "preference"},
     "Healthy skepticism. AI is a tool, not a replacement for understanding."),
    ("I'm a huge fan of immutable infrastructure.", "remember",
     {"text": "User advocates for immutable infrastructure", "importance": 0.4, "memory_type": "semantic", "domain": "preference"},
     "Cattle not pets. Good approach to operations."),
    ("I'd rather read source code than documentation.", "save_user_fact",
     {"fact": "User prefers reading source code over documentation", "domain": "preference", "confidence": 0.85},
     "Source is the truth. Documentation lies."),
    ("I think weekly sprints are too short. Biweekly is better.", "remember",
     {"text": "User prefers biweekly sprints over weekly sprints", "importance": 0.4, "memory_type": "semantic", "domain": "preference"},
     "More time to build momentum before the ceremony overhead."),
    ("I prefer tea over coffee. Specifically green tea.", "save_user_fact",
     {"fact": "User prefers green tea over coffee", "domain": "preference", "confidence": 0.9},
     "Green tea. Clean energy without the jitters."),
    ("I love automating everything. If I do it twice, I script it.", "save_user_fact",
     {"fact": "User automates repetitive tasks, scripts anything done more than once", "domain": "preference", "confidence": 0.85},
     "The laziest good habit. Automate everything."),
]


# ---------------------------------------------------------------------------
# CATEGORY 5: NOT remembering — transient queries (60 examples)
# ---------------------------------------------------------------------------
NO_REMEMBER = [
    ("Can you convert 5 miles to kilometers?", "5 miles is about 8.05 kilometers."),
    ("What's 2+2?", "4."),
    ("Tell me a joke.", "A programmer's spouse says 'go to the store, get a gallon of milk, and if they have eggs, get a dozen.' The programmer comes home with 12 gallons of milk."),
    ("How do I exit vim?", "Press Escape, then type :q! to quit without saving, or :wq to save and quit."),
    ("What time is it?", "It's shown in your system clock at the top of the screen."),
    ("What's the capital of France?", "Paris."),
    ("How many bytes in a kilobyte?", "1024 bytes in a kibibyte, 1000 in a kilobyte. Depends on which definition you're using."),
    ("Can you help me write a for loop in Python?", "Sure. `for i in range(10): print(i)` — that'll iterate 0 through 9."),
    ("What does HTTP 404 mean?", "Not Found. The server can't find the requested resource."),
    ("Remind me what REST stands for?", "Representational State Transfer."),
    ("How do I check disk space on Linux?", "Use `df -h` for filesystem usage, or `du -sh *` for directory sizes."),
    ("What's the difference between git merge and git rebase?", "Merge creates a merge commit preserving branch history. Rebase replays your commits on top of the target branch, making a linear history."),
    ("Convert 100 Celsius to Fahrenheit.", "212 degrees Fahrenheit."),
    ("What's a mutex?", "A mutual exclusion lock. Only one thread can hold it at a time. Used to protect shared resources from concurrent access."),
    ("How do I center a div in CSS?", "Use flexbox: `display: flex; justify-content: center; align-items: center;` on the parent."),
    ("What's the shortcut for undo?", "Ctrl+Z on Windows and Linux. Cmd+Z on Mac."),
    ("How many days in February this year?", "28 days. 2026 is not a leap year."),
    ("What's the square root of 144?", "12."),
    ("Explain what an API is.", "An Application Programming Interface. A defined set of rules for how software components communicate with each other."),
    ("What port does HTTPS use?", "443."),
    ("How do I list files in a directory on Linux?", "Use `ls` for a simple listing, or `ls -la` for detailed info including hidden files."),
    ("What's JSON?", "JavaScript Object Notation. A lightweight data-interchange format. Easy for humans to read and write, easy for machines to parse."),
    ("Translate 'hello' to Spanish.", "Hola."),
    ("What's the difference between == and === in JavaScript?", "== does type coercion before comparing. === compares both value and type without coercion. Always prefer ===."),
    ("How do I make a Python virtual environment?", "`python -m venv myenv` then activate it with `source myenv/bin/activate` on Unix or `myenv\\Scripts\\activate` on Windows."),
    ("What does CORS stand for?", "Cross-Origin Resource Sharing."),
    ("Help me format this date string.", "What format do you need? Common ones: `YYYY-MM-DD` for ISO 8601, `MM/DD/YYYY` for US style."),
    ("Can you count the words in this paragraph?", "Paste the paragraph and I'll count them."),
    ("What's the latest version of Node.js?", "Check nodejs.org for the current LTS. As of my knowledge, v22 is the current LTS line."),
    ("How do I make text bold in Markdown?", "Wrap it in double asterisks: `**bold text**`."),
    ("What's TCP vs UDP?", "TCP is connection-oriented, reliable, ordered. UDP is connectionless, faster, no guarantees. Use TCP when you need reliability, UDP when you need speed."),
    ("How do you spell 'necessary'?", "N-E-C-E-S-S-A-R-Y. One c, two s's."),
    ("What's the weather usually like in March?", "Depends on your location. Generally it's transitional. Spring in the northern hemisphere, autumn in the southern."),
    ("Can you proofread this sentence?", "Send it over and I'll take a look."),
    ("What's the boiling point of water?", "100 degrees Celsius, or 212 degrees Fahrenheit, at standard atmospheric pressure."),
    ("How do I install a package with pip?", "`pip install package-name`. Use `pip install -r requirements.txt` for a requirements file."),
    ("What's the difference between a stack and a queue?", "Stack is LIFO — last in, first out. Queue is FIFO — first in, first out."),
    ("Random number between 1 and 100?", "47."),
    ("What does SQL stand for?", "Structured Query Language."),
    ("How do I check my Python version?", "`python --version` or `python3 --version` in your terminal."),
    ("What's a lambda function?", "An anonymous, inline function. In Python: `lambda x: x * 2`. Useful for short, throwaway operations."),
    ("How many ounces in a cup?", "8 fluid ounces."),
    ("What's the command to create a new git branch?", "`git checkout -b branch-name` or `git switch -c branch-name`."),
    ("Can you suggest a good font for coding?", "JetBrains Mono, Fira Code, or Cascadia Code. All have good ligature support."),
    ("What's Big O notation?", "A way to describe the upper bound of an algorithm's time or space complexity. O(1) is constant, O(n) is linear, O(n^2) is quadratic, etc."),
    ("How do I grep for a pattern in files?", "`grep -r 'pattern' directory/` for recursive search. Use `-i` for case-insensitive."),
    ("What's the hex code for white?", "#FFFFFF."),
    ("How do I clear my terminal?", "`clear` on Unix, `cls` on Windows. Or Ctrl+L."),
    ("What's the difference between a class and a struct?", "Depends on the language. In C++/C#, classes default to private access, structs to public. In Rust, structs don't support inheritance. In many languages they're functionally similar."),
    ("Can you calculate 15% tip on $67?", "15% of $67 is $10.05. Total would be $77.05."),
    ("What's SSH?", "Secure Shell. An encrypted protocol for remote login and command execution. Default port 22."),
    ("How do I reverse a string in Python?", "`my_string[::-1]` using slice notation."),
    ("What's the shortcut to find and replace?", "Ctrl+H in most editors. Cmd+Shift+H in some Mac apps."),
    ("What's an ORM?", "Object-Relational Mapping. Maps database tables to code objects so you can query the database using your programming language instead of raw SQL."),
    ("How many grams in a kilogram?", "1000 grams."),
    ("What does YAML stand for?", "YAML Ain't Markup Language. Recursive acronym. Originally 'Yet Another Markup Language'."),
    ("Help me debug this error: IndexError: list index out of range.", "You're trying to access an index that doesn't exist. Check your list length with `len(my_list)` before accessing. Common with off-by-one errors."),
    ("What's a Docker container?", "A lightweight, isolated runtime environment that packages an application with its dependencies. Think of it as a lightweight VM without the OS overhead."),
    ("What are the primary colors?", "In light: red, green, blue. In paint: red, yellow, blue. In printing: cyan, magenta, yellow."),
    ("How do I rename a file in the terminal?", "`mv oldname newname` on Unix. `ren oldname newname` on Windows."),
]


# ---------------------------------------------------------------------------
# GENERATION
# ---------------------------------------------------------------------------
def generate_all():
    examples = []

    # Category 1: Casual personal facts (120)
    for i, (user_msg, fact, domain, confidence, response) in enumerate(CASUAL_FACTS):
        bond = BOND_STAGES[i % len(BOND_STAGES)]
        name = NAMES[i % len(NAMES)]
        time = TIMES[i % len(TIMES)]
        tool_args = {"fact": fact, "domain": domain, "confidence": confidence}
        tool_result = {"status": "saved", "rid": mem_id()}
        ex = make_example(user_msg, "save_user_fact", tool_args, tool_result,
                          response, bond, "casual_fact", name, time)
        examples.append(ex)

    # Category 2: Corrections (80)
    # Pad to 80 by cycling if needed
    corrections_padded = CORRECTIONS[:]
    while len(corrections_padded) < 80:
        extra = CORRECTIONS[len(corrections_padded) % len(CORRECTIONS)]
        corrections_padded.append(extra)
    corrections_padded = corrections_padded[:80]

    for i, item in enumerate(corrections_padded):
        user_msg, tool_name, tool_args, response = item
        bond = BOND_STAGES[i % len(BOND_STAGES)]
        name = NAMES[i % len(NAMES)]
        time = TIMES[i % len(TIMES)]
        if tool_name == "save_user_fact":
            tool_result = {"status": "saved", "rid": mem_id()}
        else:
            tool_result = {"status": "saved", "rid": mem_id()}
        ex = make_example(user_msg, tool_name, tool_args, tool_result,
                          response, bond, "correction", name, time)
        examples.append(ex)

    # Category 3: Events (80)
    events_padded = EVENTS[:]
    while len(events_padded) < 80:
        extra = EVENTS[len(events_padded) % len(EVENTS)]
        events_padded.append(extra)
    events_padded = events_padded[:80]

    for i, (user_msg, text, importance, memory_type, domain, response) in enumerate(events_padded):
        bond = BOND_STAGES[i % len(BOND_STAGES)]
        name = NAMES[i % len(NAMES)]
        time = TIMES[i % len(TIMES)]
        tool_args = {"text": text, "importance": importance, "memory_type": memory_type, "domain": domain}
        tool_result = {"status": "saved", "rid": mem_id()}
        ex = make_example(user_msg, "remember", tool_args, tool_result,
                          response, bond, "event", name, time)
        examples.append(ex)

    # Category 4: Preferences (60)
    prefs_padded = PREFERENCES[:]
    while len(prefs_padded) < 60:
        extra = PREFERENCES[len(prefs_padded) % len(PREFERENCES)]
        prefs_padded.append(extra)
    prefs_padded = prefs_padded[:60]

    for i, (user_msg, tool_name, tool_args, response) in enumerate(prefs_padded):
        bond = BOND_STAGES[i % len(BOND_STAGES)]
        name = NAMES[i % len(NAMES)]
        time = TIMES[i % len(TIMES)]
        if tool_name == "save_user_fact":
            tool_result = {"status": "saved", "rid": mem_id()}
        else:
            tool_result = {"status": "saved", "rid": mem_id()}
        ex = make_example(user_msg, tool_name, tool_args, tool_result,
                          response, bond, "preference", name, time)
        examples.append(ex)

    # Category 5: No tool (60)
    no_remember_padded = NO_REMEMBER[:]
    while len(no_remember_padded) < 60:
        extra = NO_REMEMBER[len(no_remember_padded) % len(NO_REMEMBER)]
        no_remember_padded.append(extra)
    no_remember_padded = no_remember_padded[:60]

    for i, (user_msg, response) in enumerate(no_remember_padded):
        bond = BOND_STAGES[i % len(BOND_STAGES)]
        name = NAMES[i % len(NAMES)]
        time = TIMES[i % len(TIMES)]
        ex = make_no_tool_example(user_msg, response, bond, "no_tool", name, time)
        examples.append(ex)

    # Shuffle
    random.shuffle(examples)

    return examples


def main():
    examples = generate_all()
    assert len(examples) == 400, f"Expected 400 examples, got {len(examples)}"

    out_dir = os.path.dirname(os.path.abspath(__file__))
    out_path = os.path.join(out_dir, "batch_memory_02_remember.jsonl")

    with open(out_path, "w", encoding="utf-8") as f:
        for ex in examples:
            f.write(json.dumps(ex, ensure_ascii=False) + "\n")

    print(f"Generated {len(examples)} examples -> {out_path}")

    # Verify counts by scenario_type
    from collections import Counter
    counts = Counter(ex["metadata"]["scenario_type"] for ex in examples)
    for k, v in sorted(counts.items()):
        print(f"  {k}: {v}")


if __name__ == "__main__":
    main()
