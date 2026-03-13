#!/usr/bin/env python3
"""
Generate synthetic training data for SCHEDULE and COMMUNICATE tool families.
Outputs:
  - batch_tools_07_schedule.jsonl   (150 examples)
  - batch_tools_08_communicate.jsonl (100 examples)
"""

import json
import random
import os
from pathlib import Path

random.seed(42)

OUT_DIR = Path(__file__).parent

# ---------------------------------------------------------------------------
# System prompts per bond stage
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
# Helper data pools
# ---------------------------------------------------------------------------
PEOPLE = [
    "Sarah", "Marcus", "Priya", "James", "Elena", "David", "Aisha", "Tom",
    "Kenji", "Olivia", "Carlos", "Nina", "Raj", "Megan", "Leo", "Fatima",
]
COMPANIES = [
    "Acme Corp", "TechVentures", "CloudPeak", "DataSync", "NovaSoft",
    "Meridian Labs", "Apex Solutions", "ByteForge", "ClearPath", "OmniTech",
]
EMAIL_DOMAINS = ["gmail.com", "outlook.com", "protonmail.com", "company.com", "work.org"]
MEETING_TOPICS = [
    "Q1 budget review", "sprint planning", "design review", "1-on-1 sync",
    "product demo", "architecture discussion", "team standup", "client call",
    "interview panel", "retrospective", "launch planning", "performance review",
    "onboarding session", "security audit review", "roadmap alignment",
]
REMINDER_TOPICS = [
    "take medication", "call the dentist", "pick up groceries", "submit expense report",
    "review pull request", "water the plants", "back up the NAS", "renew domain",
    "pay electricity bill", "check server logs", "update passport application",
    "send invoice to client", "check CI pipeline", "order new cables",
    "follow up with recruiter", "book flight", "cancel subscription trial",
    "refill water filter", "drop off dry cleaning", "charge headphones",
]
EMAIL_SUBJECTS = [
    "Q1 Revenue Numbers", "Updated API Documentation", "Meeting Reschedule",
    "Invoice #4521", "Your order has shipped", "Security Alert: New Login",
    "Weekly Digest", "Pull Request Review Needed", "Vacation Approval",
    "Conference Registration", "Contract Renewal", "Bug Report: Dashboard",
    "New Feature Proposal", "Team Lunch Friday", "Server Maintenance Window",
    "Design Mockups v3", "Customer Feedback Summary", "Release Notes v2.4",
    "Interview Feedback", "Project Status Update",
]
TELEGRAM_CONTACTS = [
    "Mom", "Dad", "Alex", "Sam", "Jordan", "Taylor", "Morgan", "Casey",
    "Jamie", "Robin", "Chris", "Pat", "Ravi", "Yuki",
]
DAYS_OF_WEEK = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"]
MONTHS_2026 = [
    "January", "February", "March", "April", "May", "June",
    "July", "August", "September", "October", "November", "December",
]
CRON_INTERVALS = [
    "every weekday at 9am", "every Monday at 8am", "every hour",
    "every 30 minutes", "daily at midnight", "weekly on Sundays at 6pm",
    "first of every month", "every 15 minutes during work hours",
]
NOTIFICATION_TITLES = [
    "Download complete", "Build succeeded", "Backup finished",
    "New message received", "System update available", "Low disk space warning",
    "Timer expired", "Meeting starting in 5 minutes", "Task overdue",
]

# ---------------------------------------------------------------------------
# Utility
# ---------------------------------------------------------------------------
def rand_date_2026():
    m = random.randint(1, 12)
    d = random.randint(1, 28)
    return f"2026-{m:02d}-{d:02d}"

def rand_time():
    h = random.randint(6, 22)
    m = random.choice([0, 15, 30, 45])
    return f"{h:02d}:{m:02d}"

def rand_datetime_2026():
    return f"{rand_date_2026()}T{rand_time()}"

def rand_duration_mins():
    return random.choice([5, 10, 15, 20, 25, 30, 45, 60, 90, 120])

def rand_email():
    name = random.choice(PEOPLE).lower()
    domain = random.choice(EMAIL_DOMAINS)
    return f"{name}@{domain}"

def rand_bond():
    return random.choice(BOND_STAGES)

def make_line(conversations, metadata):
    return json.dumps({"conversations": conversations, "metadata": metadata}, ensure_ascii=False)


# ===================================================================
# SCHEDULE TOOL EXAMPLES
# ===================================================================

def gen_schedule_examples():
    examples = []

    # ---------------------------------------------------------------
    # 1. set_reminder — basic (30 examples)
    # ---------------------------------------------------------------
    reminder_prompts = [
        ("Remind me to {topic} tomorrow at 9am", "2026-{m:02d}-{d:02d}T09:00"),
        ("Can you set a reminder for {topic} at {time}?", None),
        ("I need to remember to {topic} on {date}", None),
        ("Remind me about {topic} in 2 hours", None),
        ("Set a reminder: {topic}, next Monday at 10am", None),
        ("Don't let me forget to {topic} tonight at 8pm", None),
        ("Reminder for {topic} at noon tomorrow", None),
        ("I keep forgetting to {topic}. Remind me at {time} every day", None),
    ]
    for i in range(20):
        bond = rand_bond()
        topic = random.choice(REMINDER_TOPICS)
        dt = rand_datetime_2026()
        time_str = rand_time()
        date_str = rand_date_2026()

        user_msgs = [
            f"Remind me to {topic} tomorrow at 9am",
            f"Can you set a reminder for {topic} at {time_str}?",
            f"I need to remember to {topic} on {date_str}",
            f"Remind me about {topic} in 2 hours",
            f"Set a reminder: {topic}, next Monday at 10am",
            f"Don't let me forget to {topic} tonight at 8pm",
            f"Hey, remind me to {topic} at {time_str}",
            f"I need a reminder to {topic} by end of day",
            f"Ping me about {topic} at {time_str} on {date_str}",
            f"Reminder to {topic} this afternoon",
        ]
        user_msg = user_msgs[i % len(user_msgs)]

        assistant_prefixes = [
            f"Setting a reminder for you.",
            f"Got it. I'll remind you.",
            f"Reminder set.",
            f"Done.",
            f"I'll make sure you don't forget.",
        ]
        asst_prefix = random.choice(assistant_prefixes)

        result_msg = json.dumps({"status": "ok", "reminder_id": f"rem_{random.randint(1000,9999)}", "trigger_at": dt})

        follow_ups = [
            f"All set. I'll ping you at {dt.split('T')[1]} on {dt.split('T')[0]}.",
            f"Reminder is locked in. You'll hear from me when it's time.",
            f"Done. I'll nudge you at the right time.",
            f"You're covered. Reminder queued for {dt.split('T')[0]}.",
        ]
        follow_up = random.choice(follow_ups)

        convos = [
            {"role": "system", "content": BOND_PROMPTS[bond]},
            {"role": "user", "content": user_msg},
            {"role": "assistant", "content": asst_prefix, "tool_calls": [
                {"name": "set_reminder", "arguments": {"text": topic.capitalize(), "trigger_at": dt}}
            ]},
            {"role": "tool", "name": "set_reminder", "content": result_msg},
            {"role": "assistant", "content": follow_up},
        ]
        examples.append(make_line(convos, {
            "bond_stage": bond, "scenario_type": "task_request", "tools_used": ["set_reminder"]
        }))

    # ---------------------------------------------------------------
    # 2. timer (20 examples)
    # ---------------------------------------------------------------
    for i in range(15):
        bond = rand_bond()
        mins = rand_duration_mins()
        purposes = [
            "pomodoro break", "stretch break", "tea steeping", "code review",
            "meeting prep", "focus session", "pasta boiling", "laundry",
            "phone charge check", "eye rest", "standup reminder", "cooldown",
        ]
        purpose = random.choice(purposes)
        user_msgs = [
            f"Set a timer for {mins} minutes",
            f"Timer {mins} minutes for {purpose}",
            f"Start a {mins}-minute timer",
            f"Can you time {mins} minutes? I'm doing a {purpose}",
            f"{mins} minute timer please",
            f"I need a {purpose} timer, {mins} minutes",
        ]
        user_msg = random.choice(user_msgs)

        result = json.dumps({"status": "ok", "timer_id": f"tmr_{random.randint(100,999)}", "duration_seconds": mins * 60})

        asst_responses = [
            f"Timer started. {mins} minutes on the clock.",
            f"{mins}-minute timer running.",
            f"Started. I'll let you know when {mins} minutes are up.",
            f"Timer set for {mins} minutes.",
        ]
        convos = [
            {"role": "system", "content": BOND_PROMPTS[bond]},
            {"role": "user", "content": user_msg},
            {"role": "assistant", "content": "", "tool_calls": [
                {"name": "timer", "arguments": {"duration_seconds": mins * 60, "label": purpose}}
            ]},
            {"role": "tool", "name": "timer", "content": result},
            {"role": "assistant", "content": random.choice(asst_responses)},
        ]
        examples.append(make_line(convos, {
            "bond_stage": bond, "scenario_type": "task_request", "tools_used": ["timer"]
        }))

    # ---------------------------------------------------------------
    # 3. calendar_today (15 examples)
    # ---------------------------------------------------------------
    for i in range(10):
        bond = rand_bond()
        user_msgs = [
            "What's on my calendar today?",
            "Do I have any meetings today?",
            "What does today look like?",
            "Any events today?",
            "Show me today's schedule",
            "Am I free this afternoon?",
            "What's my day look like?",
        ]
        user_msg = random.choice(user_msgs)

        num_events = random.randint(0, 4)
        events = []
        for _ in range(num_events):
            events.append({
                "title": random.choice(MEETING_TOPICS),
                "start": f"2026-03-13T{rand_time()}",
                "end": f"2026-03-13T{rand_time()}",
                "location": random.choice(["Zoom", "Room 3B", "Google Meet", "Office", ""]),
            })
        result = json.dumps({"events": events, "date": "2026-03-13"})

        if num_events == 0:
            responses = [
                "Your calendar is clear today. No meetings, no events.",
                "Nothing on the books today. Wide open.",
                "No events scheduled for today.",
            ]
        elif num_events == 1:
            responses = [
                f"You have one thing today: {events[0]['title']} at {events[0]['start'].split('T')[1]}.",
                f"Just one event: {events[0]['title']}.",
            ]
        else:
            responses = [
                f"You have {num_events} events today. " + ", ".join(e["title"] for e in events) + ".",
                f"{num_events} things on your plate today.",
            ]

        convos = [
            {"role": "system", "content": BOND_PROMPTS[bond]},
            {"role": "user", "content": user_msg},
            {"role": "assistant", "content": "Let me check.", "tool_calls": [
                {"name": "calendar_today", "arguments": {}}
            ]},
            {"role": "tool", "name": "calendar_today", "content": result},
            {"role": "assistant", "content": random.choice(responses)},
        ]
        examples.append(make_line(convos, {
            "bond_stage": bond, "scenario_type": "info_request", "tools_used": ["calendar_today"]
        }))

    # ---------------------------------------------------------------
    # 4. calendar_list_events (12 examples)
    # ---------------------------------------------------------------
    for i in range(8):
        bond = rand_bond()
        start_date = rand_date_2026()
        end_date = f"2026-{int(start_date[5:7]):02d}-{min(28, int(start_date[8:10])+7):02d}"
        user_msgs = [
            f"What's on my calendar this week?",
            f"Show me events from {start_date} to {end_date}",
            f"Any meetings next week?",
            f"What do I have scheduled for the rest of the week?",
            f"List my calendar events for March",
        ]
        user_msg = random.choice(user_msgs)

        events = [{"title": random.choice(MEETING_TOPICS), "start": f"{start_date}T{rand_time()}", "end": f"{start_date}T{rand_time()}"} for _ in range(random.randint(1, 6))]
        result = json.dumps({"events": events, "range": {"start": start_date, "end": end_date}})

        n = len(events)
        responses = [
            f"You have {n} events in that range. " + "; ".join(f"{e['title']} on {e['start'].split('T')[0]}" for e in events[:3]) + ("..." if n > 3 else "."),
            f"Found {n} events. Here's the breakdown: " + ", ".join(e['title'] for e in events[:4]) + ".",
        ]

        convos = [
            {"role": "system", "content": BOND_PROMPTS[bond]},
            {"role": "user", "content": user_msg},
            {"role": "assistant", "content": "", "tool_calls": [
                {"name": "calendar_list_events", "arguments": {"start_date": start_date, "end_date": end_date}}
            ]},
            {"role": "tool", "name": "calendar_list_events", "content": result},
            {"role": "assistant", "content": random.choice(responses)},
        ]
        examples.append(make_line(convos, {
            "bond_stage": bond, "scenario_type": "info_request", "tools_used": ["calendar_list_events"]
        }))

    # ---------------------------------------------------------------
    # 5. calendar_create_event (15 examples)
    # ---------------------------------------------------------------
    for i in range(10):
        bond = rand_bond()
        topic = random.choice(MEETING_TOPICS)
        dt = rand_datetime_2026()
        dur = random.choice([30, 45, 60, 90])
        person = random.choice(PEOPLE)
        user_msgs = [
            f"Schedule a {topic} for {dt.split('T')[0]} at {dt.split('T')[1]}",
            f"Create a meeting: {topic} with {person}, {dt.split('T')[0]} {dt.split('T')[1]}",
            f"Add a {dur}-minute {topic} to my calendar on {dt.split('T')[0]}",
            f"Block off time for {topic} tomorrow at 2pm",
            f"Put a {topic} on my calendar for next {random.choice(DAYS_OF_WEEK)}",
        ]
        user_msg = random.choice(user_msgs)

        end_h = int(dt.split('T')[1].split(':')[0]) + dur // 60
        end_m = int(dt.split('T')[1].split(':')[1]) + dur % 60
        end_time = f"{min(end_h, 23):02d}:{end_m % 60:02d}"
        end_dt = f"{dt.split('T')[0]}T{end_time}"

        result = json.dumps({"status": "created", "event_id": f"evt_{random.randint(1000,9999)}", "title": topic, "start": dt, "end": end_dt})

        responses = [
            f"Created: {topic} on {dt.split('T')[0]} at {dt.split('T')[1]}, {dur} minutes.",
            f"Done. {topic} is on your calendar.",
            f"Event added. {dt.split('T')[0]}, {dt.split('T')[1]} to {end_time}.",
        ]

        convos = [
            {"role": "system", "content": BOND_PROMPTS[bond]},
            {"role": "user", "content": user_msg},
            {"role": "assistant", "content": "", "tool_calls": [
                {"name": "calendar_create_event", "arguments": {
                    "title": topic, "start": dt, "end": end_dt,
                }}
            ]},
            {"role": "tool", "name": "calendar_create_event", "content": result},
            {"role": "assistant", "content": random.choice(responses)},
        ]
        examples.append(make_line(convos, {
            "bond_stage": bond, "scenario_type": "task_request", "tools_used": ["calendar_create_event"]
        }))

    # ---------------------------------------------------------------
    # 6. calendar_update_event (8 examples)
    # ---------------------------------------------------------------
    for i in range(8):
        bond = rand_bond()
        event_id = f"evt_{random.randint(1000,9999)}"
        topic = random.choice(MEETING_TOPICS)
        new_time = rand_time()
        user_msgs = [
            f"Move my {topic} to {new_time}",
            f"Reschedule the {topic} to next {random.choice(DAYS_OF_WEEK)}",
            f"Change the time of {topic} to {new_time}",
            f"Push the {topic} back an hour",
        ]
        user_msg = random.choice(user_msgs)
        new_dt = f"{rand_date_2026()}T{new_time}"

        result = json.dumps({"status": "updated", "event_id": event_id, "new_start": new_dt})

        responses = [
            f"Updated. {topic} is now at {new_time}.",
            f"Moved. New time: {new_dt.replace('T', ' ')}.",
            f"Done. I've rescheduled it.",
        ]

        convos = [
            {"role": "system", "content": BOND_PROMPTS[bond]},
            {"role": "user", "content": user_msg},
            {"role": "assistant", "content": "", "tool_calls": [
                {"name": "calendar_update_event", "arguments": {"event_id": event_id, "start": new_dt}}
            ]},
            {"role": "tool", "name": "calendar_update_event", "content": result},
            {"role": "assistant", "content": random.choice(responses)},
        ]
        examples.append(make_line(convos, {
            "bond_stage": bond, "scenario_type": "task_request", "tools_used": ["calendar_update_event"]
        }))

    # ---------------------------------------------------------------
    # 7. calendar_delete_event (6 examples)
    # ---------------------------------------------------------------
    for i in range(6):
        bond = rand_bond()
        topic = random.choice(MEETING_TOPICS)
        event_id = f"evt_{random.randint(1000,9999)}"
        user_msgs = [
            f"Cancel my {topic}",
            f"Delete the {topic} from my calendar",
            f"Remove the {topic} meeting",
        ]
        user_msg = random.choice(user_msgs)
        result = json.dumps({"status": "deleted", "event_id": event_id})

        responses = [
            f"Removed. {topic} is off your calendar.",
            f"Done. It's cancelled.",
            f"Deleted the {topic} event.",
        ]

        convos = [
            {"role": "system", "content": BOND_PROMPTS[bond]},
            {"role": "user", "content": user_msg},
            {"role": "assistant", "content": "", "tool_calls": [
                {"name": "calendar_delete_event", "arguments": {"event_id": event_id}}
            ]},
            {"role": "tool", "name": "calendar_delete_event", "content": result},
            {"role": "assistant", "content": random.choice(responses)},
        ]
        examples.append(make_line(convos, {
            "bond_stage": bond, "scenario_type": "task_request", "tools_used": ["calendar_delete_event"]
        }))

    # ---------------------------------------------------------------
    # 8. date_calc (10 examples)
    # ---------------------------------------------------------------
    for i in range(10):
        bond = rand_bond()
        queries = [
            ("How many days until March 31?", {"operation": "days_until", "date": "2026-03-31"}),
            ("What day of the week is April 15, 2026?", {"operation": "day_of_week", "date": "2026-04-15"}),
            ("What's the date 30 days from now?", {"operation": "add_days", "date": "2026-03-13", "days": 30}),
            ("How many weeks until my birthday on June 20?", {"operation": "days_until", "date": "2026-06-20"}),
            ("What date is next Friday?", {"operation": "next_weekday", "weekday": "friday"}),
            ("How many business days between March 13 and April 1?", {"operation": "business_days", "start": "2026-03-13", "end": "2026-04-01"}),
            ("What was the date 2 weeks ago?", {"operation": "add_days", "date": "2026-03-13", "days": -14}),
            ("Is 2026 a leap year?", {"operation": "is_leap_year", "year": 2026}),
            ("How many days are in February 2026?", {"operation": "days_in_month", "year": 2026, "month": 2}),
            ("What quarter is April in?", {"operation": "quarter", "month": 4}),
        ]
        query, args = queries[i]
        results = [
            '{"result": 18, "unit": "days"}',
            '{"result": "Wednesday"}',
            '{"result": "2026-04-12"}',
            '{"result": 99, "unit": "days"}',
            '{"result": "2026-03-20"}',
            '{"result": 13, "unit": "business_days"}',
            '{"result": "2026-02-27"}',
            '{"result": false}',
            '{"result": 28}',
            '{"result": "Q2"}',
        ]
        result = results[i]
        answers = [
            "18 days from now.",
            "April 15, 2026 falls on a Wednesday.",
            "30 days from today is April 12, 2026.",
            "99 days until June 20.",
            "Next Friday is March 20.",
            "13 business days between those dates.",
            "Two weeks ago was February 27.",
            "No, 2026 is not a leap year.",
            "February 2026 has 28 days.",
            "April is in Q2.",
        ]

        convos = [
            {"role": "system", "content": BOND_PROMPTS[bond]},
            {"role": "user", "content": query},
            {"role": "assistant", "content": "", "tool_calls": [
                {"name": "date_calc", "arguments": args}
            ]},
            {"role": "tool", "name": "date_calc", "content": result},
            {"role": "assistant", "content": answers[i]},
        ]
        examples.append(make_line(convos, {
            "bond_stage": bond, "scenario_type": "info_request", "tools_used": ["date_calc"]
        }))

    # ---------------------------------------------------------------
    # 9. schedule_task / create_schedule (10 examples)
    # ---------------------------------------------------------------
    for i in range(8):
        bond = rand_bond()
        task_descs = [
            ("Run backup every night at 2am", "0 2 * * *", "system_backup"),
            ("Check disk space every 6 hours", "0 */6 * * *", "disk_check"),
            ("Send me a daily summary at 8am", "0 8 * * *", "daily_summary"),
            ("Clean temp files every Sunday", "0 3 * * 0", "temp_cleanup"),
            ("Check for updates every Monday at 9am", "0 9 * * 1", "update_check"),
            ("Rotate logs weekly", "0 0 * * 0", "log_rotation"),
            ("Monitor CPU usage every 5 minutes", "*/5 * * * *", "cpu_monitor"),
            ("Fetch RSS feeds every hour", "0 * * * *", "rss_fetch"),
            ("Archive old emails monthly", "0 0 1 * *", "email_archive"),
            ("Run health check every 15 minutes", "*/15 * * * *", "health_check"),
        ]
        desc, cron, name = task_descs[i]
        user_msgs = [
            f"{desc}",
            f"Schedule a task: {desc.lower()}",
            f"Can you set up a recurring task to {desc.lower()}?",
        ]
        user_msg = random.choice(user_msgs)

        # Alternate between schedule_task and create_schedule
        tool_name = "schedule_task" if i % 2 == 0 else "create_schedule"
        result = json.dumps({"status": "scheduled", "schedule_id": f"sch_{random.randint(100,999)}", "cron": cron, "name": name})

        responses = [
            f"Scheduled. {desc} ({cron}).",
            f"Done. That'll run on schedule: {cron}.",
            f"Set up. The task is registered and will execute on the cron pattern {cron}.",
        ]

        convos = [
            {"role": "system", "content": BOND_PROMPTS[bond]},
            {"role": "user", "content": user_msg},
            {"role": "assistant", "content": "", "tool_calls": [
                {"name": tool_name, "arguments": {"name": name, "cron": cron, "command": name}}
            ]},
            {"role": "tool", "name": tool_name, "content": result},
            {"role": "assistant", "content": random.choice(responses)},
        ]
        examples.append(make_line(convos, {
            "bond_stage": bond, "scenario_type": "task_request", "tools_used": [tool_name]
        }))

    # ---------------------------------------------------------------
    # 10. list_scheduled / list_schedules (6 examples)
    # ---------------------------------------------------------------
    for i in range(7):
        bond = rand_bond()
        user_msgs = [
            "What tasks do I have scheduled?",
            "Show me my scheduled tasks",
            "List all scheduled jobs",
            "What's running on a schedule?",
            "Any recurring tasks set up?",
            "Show my cron jobs",
            "What automated jobs are active?",
        ]
        user_msg = user_msgs[i]
        tool_name = "list_scheduled" if i % 2 == 0 else "list_schedules"

        tasks = [
            {"name": "daily_summary", "cron": "0 8 * * *", "next_run": "2026-03-14T08:00"},
            {"name": "backup", "cron": "0 2 * * *", "next_run": "2026-03-14T02:00"},
            {"name": "disk_check", "cron": "0 */6 * * *", "next_run": "2026-03-13T18:00"},
        ]
        n = random.randint(0, 3)
        result = json.dumps({"schedules": tasks[:n]})

        if n == 0:
            resp = "No scheduled tasks right now."
        else:
            resp = f"You have {n} scheduled task{'s' if n > 1 else ''}: " + ", ".join(t["name"] for t in tasks[:n]) + "."

        convos = [
            {"role": "system", "content": BOND_PROMPTS[bond]},
            {"role": "user", "content": user_msg},
            {"role": "assistant", "content": "", "tool_calls": [
                {"name": tool_name, "arguments": {}}
            ]},
            {"role": "tool", "name": tool_name, "content": result},
            {"role": "assistant", "content": resp},
        ]
        examples.append(make_line(convos, {
            "bond_stage": bond, "scenario_type": "info_request", "tools_used": [tool_name]
        }))

    # ---------------------------------------------------------------
    # 11. cancel_scheduled / cancel_schedule (5 examples)
    # ---------------------------------------------------------------
    for i in range(3):
        bond = rand_bond()
        names = ["daily_summary", "backup", "disk_check", "log_rotation", "health_check"]
        name = names[i]
        sch_id = f"sch_{random.randint(100,999)}"
        user_msgs = [
            f"Cancel the {name} schedule",
            f"Stop the {name} task",
            f"Remove the {name} from scheduled tasks",
            f"Delete the {name} cron job",
            f"Turn off the {name} schedule",
        ]
        user_msg = user_msgs[i]
        tool_name = "cancel_scheduled" if i % 2 == 0 else "cancel_schedule"

        result = json.dumps({"status": "cancelled", "schedule_id": sch_id})
        responses = [
            f"Cancelled the {name} schedule.",
            f"Done. {name} won't run anymore.",
            f"Removed. The {name} schedule is gone.",
        ]

        convos = [
            {"role": "system", "content": BOND_PROMPTS[bond]},
            {"role": "user", "content": user_msg},
            {"role": "assistant", "content": "", "tool_calls": [
                {"name": tool_name, "arguments": {"schedule_id": sch_id}}
            ]},
            {"role": "tool", "name": tool_name, "content": result},
            {"role": "assistant", "content": random.choice(responses)},
        ]
        examples.append(make_line(convos, {
            "bond_stage": bond, "scenario_type": "task_request", "tools_used": [tool_name]
        }))

    # ---------------------------------------------------------------
    # 12. update_schedule (4 examples)
    # ---------------------------------------------------------------
    for i in range(5):
        bond = rand_bond()
        sch_id = f"sch_{random.randint(100,999)}"
        name = random.choice(["backup", "daily_summary", "disk_check", "health_check", "rss_fetch"])
        new_cron = random.choice(["0 3 * * *", "0 9 * * 1-5", "*/30 * * * *", "0 0 * * 0", "0 */4 * * *"])
        user_msgs = [
            f"Change the {name} schedule to run at 3am instead",
            f"Update the {name} to {new_cron}",
            f"Make the {name} run on weekdays only",
            f"Switch the {name} to every 30 minutes",
            f"Change {name} to run every 4 hours",
        ]
        user_msg = user_msgs[i]

        result = json.dumps({"status": "updated", "schedule_id": sch_id, "new_cron": new_cron})
        resp = f"Updated. {name} now runs on schedule: {new_cron}."

        convos = [
            {"role": "system", "content": BOND_PROMPTS[bond]},
            {"role": "user", "content": user_msg},
            {"role": "assistant", "content": "", "tool_calls": [
                {"name": "update_schedule", "arguments": {"schedule_id": sch_id, "cron": new_cron}}
            ]},
            {"role": "tool", "name": "update_schedule", "content": result},
            {"role": "assistant", "content": resp},
        ]
        examples.append(make_line(convos, {
            "bond_stage": bond, "scenario_type": "task_request", "tools_used": ["update_schedule"]
        }))

    # ---------------------------------------------------------------
    # 13. Multi-tool: calendar_today + set_reminder (15 examples)
    # ---------------------------------------------------------------
    for i in range(15):
        bond = rand_bond()
        user_msgs = [
            "What's on today? And remind me to prep for the first meeting 30 minutes before",
            "Check my calendar and set a reminder for the most important meeting",
            "Show today's events and remind me to leave for the office at 8am",
            "Any meetings today? Also remind me to submit my timesheet at 5pm",
            "What does today look like, and set a reminder to grab lunch at noon",
            "Check my schedule and set a 15-minute prep reminder before each meeting",
            "Today's calendar please. Also remind me to call the electrician at 3pm",
            "Show me what I have today, and remind me about the dentist at 4pm",
            "Pull up today's schedule. Remind me to pick up my prescription after work",
            "Calendar check plus a reminder to review the PR before standup",
            "What meetings do I have? Also set a reminder to prep slides at 1pm",
            "Show me today's events and remind me to water the plants at 6pm",
            "Check today and set a reminder to send the weekly report at 4pm",
            "Today's agenda plus a reminder to charge my laptop before the demo",
            "What's happening today? Remind me to follow up with the client at 11am",
        ]
        user_msg = user_msgs[i]

        events = [{"title": random.choice(MEETING_TOPICS), "start": f"2026-03-13T{rand_time()}", "end": f"2026-03-13T{rand_time()}"} for _ in range(random.randint(1, 3))]
        cal_result = json.dumps({"events": events, "date": "2026-03-13"})

        reminder_topic = random.choice(REMINDER_TOPICS)
        rem_dt = rand_datetime_2026()
        rem_result = json.dumps({"status": "ok", "reminder_id": f"rem_{random.randint(1000,9999)}", "trigger_at": rem_dt})

        n = len(events)
        final = f"You have {n} event{'s' if n > 1 else ''} today: {', '.join(e['title'] for e in events)}. Reminder set for {rem_dt.split('T')[1]}."

        convos = [
            {"role": "system", "content": BOND_PROMPTS[bond]},
            {"role": "user", "content": user_msg},
            {"role": "assistant", "content": "Let me check your calendar and set that reminder.", "tool_calls": [
                {"name": "calendar_today", "arguments": {}},
                {"name": "set_reminder", "arguments": {"text": reminder_topic.capitalize(), "trigger_at": rem_dt}},
            ]},
            {"role": "tool", "name": "calendar_today", "content": cal_result},
            {"role": "tool", "name": "set_reminder", "content": rem_result},
            {"role": "assistant", "content": final},
        ]
        examples.append(make_line(convos, {
            "bond_stage": bond, "scenario_type": "multi_tool", "tools_used": ["calendar_today", "set_reminder"]
        }))

    # ---------------------------------------------------------------
    # 14. Multi-tool: calendar_list_events + calendar_create_event (12 examples)
    # ---------------------------------------------------------------
    for i in range(12):
        bond = rand_bond()
        topic = random.choice(MEETING_TOPICS)
        date = rand_date_2026()
        time = rand_time()
        user_msgs = [
            f"Am I free on {date} at {time}? If so, schedule a {topic}",
            f"Check if {date} works for a {topic} and book it",
            f"Find an open slot this week for a {topic}",
            f"Is my afternoon free on {date}? I need to add a {topic}",
        ]
        user_msg = random.choice(user_msgs)

        existing = [{"title": random.choice(MEETING_TOPICS), "start": f"{date}T{rand_time()}", "end": f"{date}T{rand_time()}"} for _ in range(random.randint(0, 2))]
        list_result = json.dumps({"events": existing, "range": {"start": date, "end": date}})

        dt = f"{date}T{time}"
        end_h = min(23, int(time.split(':')[0]) + 1)
        end_dt = f"{date}T{end_h:02d}:{time.split(':')[1]}"
        create_result = json.dumps({"status": "created", "event_id": f"evt_{random.randint(1000,9999)}", "title": topic, "start": dt, "end": end_dt})

        if len(existing) == 0:
            final = f"You're free on {date}. I've added {topic} at {time}."
        else:
            final = f"You have {len(existing)} existing event{'s' if len(existing) > 1 else ''}, but {time} is open. {topic} is now on your calendar."

        convos = [
            {"role": "system", "content": BOND_PROMPTS[bond]},
            {"role": "user", "content": user_msg},
            {"role": "assistant", "content": "Checking your calendar for that slot.", "tool_calls": [
                {"name": "calendar_list_events", "arguments": {"start_date": date, "end_date": date}},
            ]},
            {"role": "tool", "name": "calendar_list_events", "content": list_result},
            {"role": "assistant", "content": f"The slot looks open. Adding it now.", "tool_calls": [
                {"name": "calendar_create_event", "arguments": {"title": topic, "start": dt, "end": end_dt}},
            ]},
            {"role": "tool", "name": "calendar_create_event", "content": create_result},
            {"role": "assistant", "content": final},
        ]
        examples.append(make_line(convos, {
            "bond_stage": bond, "scenario_type": "multi_tool", "tools_used": ["calendar_list_events", "calendar_create_event"]
        }))

    # ---------------------------------------------------------------
    # 15. Error handling examples (15 examples)
    # ---------------------------------------------------------------
    error_scenarios = [
        # Calendar service down
        ("What's on my calendar?", "calendar_today", {},
         '{"error": "calendar_service_unavailable", "message": "Could not connect to calendar service"}',
         "The calendar service isn't responding right now. I'll try again in a few minutes, or you can check manually."),
        # Invalid date for reminder
        ("Remind me to call dentist on February 30", "set_reminder", {"text": "Call dentist", "trigger_at": "2026-02-30T09:00"},
         '{"error": "invalid_date", "message": "February 30 is not a valid date"}',
         "February only has 28 days in 2026. Did you mean February 28 or March 2?"),
        # Past date reminder
        ("Set a reminder for yesterday", "set_reminder", {"text": "Reminder", "trigger_at": "2026-03-12T09:00"},
         '{"error": "past_date", "message": "Cannot set reminder in the past"}',
         "That date has already passed. Want me to set it for tomorrow instead?"),
        # Event not found for update
        ("Move my dentist appointment to 3pm", "calendar_update_event", {"event_id": "evt_0000", "start": "2026-03-13T15:00"},
         '{"error": "not_found", "message": "Event not found"}',
         "I couldn't find a dentist appointment on your calendar. Want me to search for it, or should I create a new one?"),
        # Timer already running
        ("Start a 25-minute timer", "timer", {"duration_seconds": 1500, "label": "focus"},
         '{"error": "timer_conflict", "message": "A timer is already running (12 minutes remaining)"}',
         "You already have a timer running with 12 minutes left. Want me to cancel it and start a new one, or wait until it finishes?"),
        # Permission denied for schedule
        ("Schedule a system backup at midnight", "schedule_task", {"name": "backup", "cron": "0 0 * * *", "command": "backup"},
         '{"error": "permission_denied", "message": "Insufficient permissions to schedule system tasks"}',
         "I don't have permission to schedule system-level tasks. You'll need to grant elevated access in settings first."),
        # Calendar full
        ("Add a meeting at 2pm", "calendar_create_event", {"title": "Meeting", "start": "2026-03-13T14:00", "end": "2026-03-13T15:00"},
         '{"error": "conflict", "message": "Time slot conflicts with existing event: Sprint Planning (14:00-15:30)"}',
         "That slot conflicts with Sprint Planning from 2:00 to 3:30. Want me to find the next free slot?"),
        # Network error on calendar
        ("Show me next week's meetings", "calendar_list_events", {"start_date": "2026-03-16", "end_date": "2026-03-22"},
         '{"error": "network_error", "message": "Network timeout while syncing calendar"}',
         "Network timed out while syncing. I can show you the last cached version, or try again when the connection is better."),
        # Invalid cron expression
        ("Run a task every purple moon", "create_schedule", {"name": "weird_task", "cron": "invalid", "command": "task"},
         '{"error": "invalid_cron", "message": "Could not parse cron expression"}',
         "I couldn't parse that into a valid schedule. Can you be more specific? Something like 'every Monday at 9am' or 'every 2 hours'."),
        # Schedule not found for cancel
        ("Cancel the log rotation schedule", "cancel_scheduled", {"schedule_id": "sch_000"},
         '{"error": "not_found", "message": "Schedule not found"}',
         "I couldn't find a log rotation schedule. Want me to list all active schedules so you can pick the right one?"),
        # Reminder storage full
        ("Set a reminder for every day this month", "set_reminder", {"text": "Daily check", "trigger_at": "2026-03-14T09:00"},
         '{"error": "quota_exceeded", "message": "Maximum reminder limit (100) reached"}',
         "You've hit the reminder limit of 100. Want me to clear out some old ones to make room?"),
        # Calendar auth expired
        ("What's on my calendar this week?", "calendar_list_events", {"start_date": "2026-03-13", "end_date": "2026-03-20"},
         '{"error": "auth_expired", "message": "Calendar authentication token expired. Please re-authenticate."}',
         "Your calendar authentication has expired. You'll need to re-connect your calendar account in Settings to fix this."),
        # Timer duration too long
        ("Set a timer for 48 hours", "timer", {"duration_seconds": 172800, "label": "long timer"},
         '{"error": "duration_exceeded", "message": "Maximum timer duration is 24 hours"}',
         "Timers max out at 24 hours. For something that far out, a reminder would work better. Want me to set a reminder for 48 hours from now?"),
        # Event update with no changes
        ("Update my meeting", "calendar_update_event", {"event_id": "evt_1234"},
         '{"error": "no_changes", "message": "No changes specified"}',
         "What would you like to change about the meeting? I can update the time, title, or duration."),
        # Schedule already exists
        ("Schedule a daily backup at 2am", "create_schedule", {"name": "backup", "cron": "0 2 * * *", "command": "backup"},
         '{"error": "duplicate", "message": "A schedule named \'backup\' already exists"}',
         "There's already a backup schedule. Want me to update its timing, or create a separate one with a different name?"),
    ]

    for i, (user_msg, tool_name, args, error_result, response) in enumerate(error_scenarios):
        bond = rand_bond()
        convos = [
            {"role": "system", "content": BOND_PROMPTS[bond]},
            {"role": "user", "content": user_msg},
            {"role": "assistant", "content": "", "tool_calls": [
                {"name": tool_name, "arguments": args}
            ]},
            {"role": "tool", "name": tool_name, "content": error_result},
            {"role": "assistant", "content": response},
        ]
        examples.append(make_line(convos, {
            "bond_stage": bond, "scenario_type": "error_handling", "tools_used": [tool_name]
        }))

    # Pad to 150 if needed with additional set_reminder/timer/calendar variations
    while len(examples) < 150:
        bond = rand_bond()
        topic = random.choice(REMINDER_TOPICS)
        dt = rand_datetime_2026()
        user_msg = f"Remind me to {topic} at {dt.split('T')[1]} on {dt.split('T')[0]}"
        result = json.dumps({"status": "ok", "reminder_id": f"rem_{random.randint(1000,9999)}", "trigger_at": dt})
        resp = f"Done. Reminder set for {dt.replace('T', ' ')}."
        convos = [
            {"role": "system", "content": BOND_PROMPTS[bond]},
            {"role": "user", "content": user_msg},
            {"role": "assistant", "content": "", "tool_calls": [
                {"name": "set_reminder", "arguments": {"text": topic.capitalize(), "trigger_at": dt}}
            ]},
            {"role": "tool", "name": "set_reminder", "content": result},
            {"role": "assistant", "content": resp},
        ]
        examples.append(make_line(convos, {
            "bond_stage": bond, "scenario_type": "task_request", "tools_used": ["set_reminder"]
        }))

    return examples[:150]


# ===================================================================
# COMMUNICATE TOOL EXAMPLES
# ===================================================================

def gen_communicate_examples():
    examples = []

    # ---------------------------------------------------------------
    # 1. email_check (10 examples)
    # ---------------------------------------------------------------
    for i in range(10):
        bond = rand_bond()
        user_msgs = [
            "Do I have any new emails?",
            "Check my email",
            "Any new messages?",
            "Has anything come in?",
            "Check for new mail",
            "Any emails?",
            "What's in my inbox?",
            "Anything new in email?",
            "Did I get any emails?",
            "Check inbox",
        ]
        user_msg = user_msgs[i]

        count = random.randint(0, 8)
        senders = [{"from": rand_email(), "subject": random.choice(EMAIL_SUBJECTS)} for _ in range(min(count, 3))]
        result = json.dumps({"new_count": count, "recent": senders})

        if count == 0:
            resp = "Inbox is clear. No new emails."
        elif count == 1:
            resp = f"One new email from {senders[0]['from']}: \"{senders[0]['subject']}\"."
        else:
            resp = f"You have {count} new emails. Most recent: " + "; ".join(f"{s['from']} -- {s['subject']}" for s in senders) + "."

        convos = [
            {"role": "system", "content": BOND_PROMPTS[bond]},
            {"role": "user", "content": user_msg},
            {"role": "assistant", "content": "", "tool_calls": [
                {"name": "email_check", "arguments": {}}
            ]},
            {"role": "tool", "name": "email_check", "content": result},
            {"role": "assistant", "content": resp},
        ]
        examples.append(make_line(convos, {
            "bond_stage": bond, "scenario_type": "info_request", "tools_used": ["email_check"]
        }))

    # ---------------------------------------------------------------
    # 2. email_list (8 examples)
    # ---------------------------------------------------------------
    for i in range(8):
        bond = rand_bond()
        folders = ["inbox", "sent", "drafts", "archive", "spam"]
        folder = random.choice(folders)
        user_msgs = [
            f"Show me my {folder} emails",
            f"List emails in {folder}",
            f"What's in my {folder}?",
            f"Show {folder}",
            "List my recent emails",
            "Show me the last 5 emails",
            "What emails do I have?",
            f"Open {folder}",
        ]
        user_msg = user_msgs[i]

        emails = [{"id": f"msg_{random.randint(1000,9999)}", "from": rand_email(), "subject": random.choice(EMAIL_SUBJECTS), "date": rand_date_2026(), "read": random.choice([True, False])} for _ in range(random.randint(1, 5))]
        result = json.dumps({"folder": folder, "emails": emails, "total": len(emails)})

        lines = [f"- {e['subject']} from {e['from']}{' (unread)' if not e['read'] else ''}" for e in emails]
        resp = f"{len(emails)} emails in {folder}:\n" + "\n".join(lines)

        convos = [
            {"role": "system", "content": BOND_PROMPTS[bond]},
            {"role": "user", "content": user_msg},
            {"role": "assistant", "content": "", "tool_calls": [
                {"name": "email_list", "arguments": {"folder": folder}}
            ]},
            {"role": "tool", "name": "email_list", "content": result},
            {"role": "assistant", "content": resp},
        ]
        examples.append(make_line(convos, {
            "bond_stage": bond, "scenario_type": "info_request", "tools_used": ["email_list"]
        }))

    # ---------------------------------------------------------------
    # 3. email_read (10 examples)
    # ---------------------------------------------------------------
    for i in range(10):
        bond = rand_bond()
        msg_id = f"msg_{random.randint(1000,9999)}"
        sender = random.choice(PEOPLE)
        sender_email = f"{sender.lower()}@{random.choice(EMAIL_DOMAINS)}"
        subject = random.choice(EMAIL_SUBJECTS)
        bodies = [
            f"Hi, just wanted to follow up on our conversation about the project timeline. Can we meet tomorrow to discuss? Best, {sender}",
            f"The invoice is attached. Payment due by end of month. Let me know if you have questions. - {sender}",
            f"Hey, the pull request is ready for review. I've addressed all the comments from last time. Link: https://github.com/example/repo/pull/42",
            f"Reminder: the team lunch is this Friday at noon. Please RSVP by Wednesday. Cheers, {sender}",
            f"Your order #A12345 has shipped and should arrive by {rand_date_2026()}. Track it here: https://tracking.example.com/A12345",
            f"The server migration is scheduled for this Saturday at 2am. Expected downtime: 30 minutes. Please plan accordingly.",
            f"Thanks for the great work on the release. The client is really happy with the results. Let's discuss next steps Monday.",
            f"Security notice: A new login was detected from an unfamiliar device. If this wasn't you, please change your password immediately.",
            f"The quarterly report is ready for your review. Key highlights: revenue up 12%, churn down 3%. Full report attached.",
            f"Hi, I'm reaching out about the open position. I'd love to schedule a call to discuss. Available this week? - {sender}",
        ]
        body = bodies[i]

        user_msgs = [
            f"Read the email from {sender}",
            f"Open the email about {subject}",
            f"What does the email from {sender} say?",
            f"Show me that email about {subject}",
            f"Read message {msg_id}",
            f"Open the latest email",
            f"What's in the email from {sender_email}?",
            f"Read the {subject} email",
            f"Pull up that email from {sender}",
            f"Show me the {subject} message",
        ]
        user_msg = user_msgs[i]

        result = json.dumps({"id": msg_id, "from": sender_email, "subject": subject, "body": body, "date": rand_date_2026(), "attachments": []})

        resp = f"From {sender} ({sender_email}):\nSubject: {subject}\n\n{body}"

        convos = [
            {"role": "system", "content": BOND_PROMPTS[bond]},
            {"role": "user", "content": user_msg},
            {"role": "assistant", "content": "", "tool_calls": [
                {"name": "email_read", "arguments": {"message_id": msg_id}}
            ]},
            {"role": "tool", "name": "email_read", "content": result},
            {"role": "assistant", "content": resp},
        ]
        examples.append(make_line(convos, {
            "bond_stage": bond, "scenario_type": "info_request", "tools_used": ["email_read"]
        }))

    # ---------------------------------------------------------------
    # 4. email_send (10 examples)
    # ---------------------------------------------------------------
    for i in range(10):
        bond = rand_bond()
        to = rand_email()
        to_name = random.choice(PEOPLE)
        subjects = [
            "Meeting follow-up", "Quick question", "Project update",
            "Document attached", "Scheduling request", "Thank you",
            "Feedback on proposal", "Availability next week",
            "Request for information", "Action items from today",
        ]
        subject = subjects[i]
        email_bodies = [
            f"Hi {to_name}, following up on our meeting today. Let me know if you need anything else.",
            f"Hey {to_name}, quick question -- do you have the latest version of the spec document?",
            f"Hi {to_name}, here's a quick update on the project status. We're on track for the deadline.",
            f"Hi {to_name}, please find the document attached. Let me know if you have any questions.",
            f"Hi {to_name}, would you be available for a 30-minute call next {random.choice(DAYS_OF_WEEK)}?",
            f"Hi {to_name}, thank you for your help with the deployment yesterday. It went smoothly.",
            f"Hi {to_name}, I've reviewed the proposal. A few notes: the timeline looks tight but the scope is right.",
            f"Hi {to_name}, are you free next week for a sync? I have openings on {random.choice(DAYS_OF_WEEK)} and {random.choice(DAYS_OF_WEEK)}.",
            f"Hi {to_name}, could you send me the access credentials for the staging environment?",
            f"Hi {to_name}, here are the action items from today's meeting: 1) finalize spec, 2) update timeline, 3) schedule review.",
        ]
        body = email_bodies[i]

        user_msgs = [
            f"Send an email to {to} about {subject.lower()}",
            f"Email {to_name} at {to}: {subject}",
            f"Write an email to {to} saying {body[:50]}...",
            f"Send {to_name} a quick email about {subject.lower()}",
            f"Compose an email to {to} with subject \"{subject}\"",
            f"Fire off an email to {to_name} about {subject.lower()}",
            f"Draft and send an email to {to}: {subject}",
            f"Email {to} -- subject: {subject}",
            f"Send a message to {to_name} ({to}) about {subject.lower()}",
            f"Shoot {to_name} an email about {subject.lower()}",
        ]
        user_msg = user_msgs[i]

        result = json.dumps({"status": "sent", "message_id": f"msg_{random.randint(1000,9999)}", "to": to, "subject": subject})

        responses = [
            f"Sent to {to}.",
            f"Email delivered to {to_name}.",
            f"Done. Message sent to {to}.",
            f"Sent. Subject: \"{subject}\".",
        ]

        convos = [
            {"role": "system", "content": BOND_PROMPTS[bond]},
            {"role": "user", "content": user_msg},
            {"role": "assistant", "content": "", "tool_calls": [
                {"name": "email_send", "arguments": {"to": to, "subject": subject, "body": body}}
            ]},
            {"role": "tool", "name": "email_send", "content": result},
            {"role": "assistant", "content": random.choice(responses)},
        ]
        examples.append(make_line(convos, {
            "bond_stage": bond, "scenario_type": "task_request", "tools_used": ["email_send"]
        }))

    # ---------------------------------------------------------------
    # 5. email_reply (8 examples)
    # ---------------------------------------------------------------
    for i in range(8):
        bond = rand_bond()
        msg_id = f"msg_{random.randint(1000,9999)}"
        sender = random.choice(PEOPLE)
        original_subject = random.choice(EMAIL_SUBJECTS)
        reply_bodies = [
            f"Thanks {sender}, that works for me. See you then.",
            f"Got it. I'll review and get back to you by end of day.",
            f"Sounds good. Let me check my calendar and confirm.",
            f"Thanks for the heads up. I'll prepare for the migration.",
            f"Looks great. I've approved the PR. Nice work.",
            f"I'll be there. Thanks for organizing.",
            f"Can we push this to next week? I'm swamped this week.",
            f"Received, thanks. I'll process the invoice today.",
        ]
        reply_body = reply_bodies[i]
        user_msgs = [
            f"Reply to {sender}'s email: {reply_body}",
            f"Reply to the {original_subject} email with: {reply_body}",
            f"Send a reply to {sender}: {reply_body}",
            f"Respond to that email from {sender}",
            f"Reply to message {msg_id}: {reply_body}",
            f"Answer {sender}'s email about {original_subject}",
            f"Write back to {sender}: {reply_body}",
            f"Reply to the last email from {sender}",
        ]
        user_msg = user_msgs[i]

        result = json.dumps({"status": "sent", "message_id": f"msg_{random.randint(1000,9999)}", "in_reply_to": msg_id})
        responses = [
            f"Reply sent to {sender}.",
            f"Done. Reply delivered.",
            f"Replied to {sender}'s email.",
        ]

        convos = [
            {"role": "system", "content": BOND_PROMPTS[bond]},
            {"role": "user", "content": user_msg},
            {"role": "assistant", "content": "", "tool_calls": [
                {"name": "email_reply", "arguments": {"message_id": msg_id, "body": reply_body}}
            ]},
            {"role": "tool", "name": "email_reply", "content": result},
            {"role": "assistant", "content": random.choice(responses)},
        ]
        examples.append(make_line(convos, {
            "bond_stage": bond, "scenario_type": "task_request", "tools_used": ["email_reply"]
        }))

    # ---------------------------------------------------------------
    # 6. email_search (6 examples)
    # ---------------------------------------------------------------
    for i in range(6):
        bond = rand_bond()
        search_queries = [
            "invoice from Acme Corp",
            "meeting notes from last week",
            "emails from Sarah about the project",
            "security alerts",
            "unread emails from this month",
            "emails with attachments",
        ]
        query = search_queries[i]
        user_msgs = [
            f"Search my emails for {query}",
            f"Find emails about {query}",
            f"Look for {query} in my email",
            f"Search for {query}",
            f"Any emails matching {query}?",
            f"Find me the {query}",
        ]
        user_msg = user_msgs[i]

        results_list = [{"id": f"msg_{random.randint(1000,9999)}", "from": rand_email(), "subject": random.choice(EMAIL_SUBJECTS), "date": rand_date_2026(), "snippet": "..."} for _ in range(random.randint(0, 4))]
        result = json.dumps({"query": query, "results": results_list, "total": len(results_list)})

        if len(results_list) == 0:
            resp = f"No emails found matching \"{query}\"."
        else:
            resp = f"Found {len(results_list)} email{'s' if len(results_list) > 1 else ''}: " + "; ".join(f"{r['subject']} from {r['from']}" for r in results_list) + "."

        convos = [
            {"role": "system", "content": BOND_PROMPTS[bond]},
            {"role": "user", "content": user_msg},
            {"role": "assistant", "content": "", "tool_calls": [
                {"name": "email_search", "arguments": {"query": query}}
            ]},
            {"role": "tool", "name": "email_search", "content": result},
            {"role": "assistant", "content": resp},
        ]
        examples.append(make_line(convos, {
            "bond_stage": bond, "scenario_type": "info_request", "tools_used": ["email_search"]
        }))

    # ---------------------------------------------------------------
    # 7. telegram_send (8 examples)
    # ---------------------------------------------------------------
    for i in range(8):
        bond = rand_bond()
        contact = random.choice(TELEGRAM_CONTACTS)
        messages = [
            f"Hey, running 10 minutes late",
            f"Can you pick up milk on the way home?",
            f"Meeting pushed to 3pm",
            f"Thanks for dinner last night!",
            f"Are we still on for tomorrow?",
            f"Just landed. Will call you in 30",
            f"The package arrived. Thanks!",
            f"Happy birthday!",
        ]
        msg = messages[i]
        user_msgs = [
            f"Send a telegram to {contact}: {msg}",
            f"Message {contact} on Telegram: {msg}",
            f"Text {contact} on Telegram saying {msg}",
            f"Telegram {contact}: {msg}",
            f"Send {contact} a message: {msg}",
            f"Tell {contact} on Telegram: {msg}",
            f"Drop {contact} a Telegram message: {msg}",
            f"Ping {contact} on Telegram: {msg}",
        ]
        user_msg = user_msgs[i]

        result = json.dumps({"status": "sent", "chat_id": f"tg_{random.randint(100000,999999)}", "contact": contact})
        responses = [
            f"Sent to {contact} on Telegram.",
            f"Message delivered to {contact}.",
            f"Done. {contact} should see it shortly.",
        ]

        convos = [
            {"role": "system", "content": BOND_PROMPTS[bond]},
            {"role": "user", "content": user_msg},
            {"role": "assistant", "content": "", "tool_calls": [
                {"name": "telegram_send", "arguments": {"contact": contact, "message": msg}}
            ]},
            {"role": "tool", "name": "telegram_send", "content": result},
            {"role": "assistant", "content": random.choice(responses)},
        ]
        examples.append(make_line(convos, {
            "bond_stage": bond, "scenario_type": "task_request", "tools_used": ["telegram_send"]
        }))

    # ---------------------------------------------------------------
    # 8. whatsapp_send (6 examples)
    # ---------------------------------------------------------------
    for i in range(6):
        bond = rand_bond()
        contact = random.choice(PEOPLE)
        messages = [
            "On my way!",
            "Can we reschedule to Thursday?",
            "Got your message, will reply properly later",
            "The document is ready, sending it over now",
            "Sure, that works for me",
            "See you at 7!",
        ]
        msg = messages[i]
        user_msgs = [
            f"WhatsApp {contact}: {msg}",
            f"Send a WhatsApp to {contact}: {msg}",
            f"Message {contact} on WhatsApp: {msg}",
            f"Text {contact} via WhatsApp: {msg}",
            f"Drop {contact} a WhatsApp: {msg}",
            f"Send {contact} a WhatsApp message saying {msg}",
        ]
        user_msg = user_msgs[i]

        result = json.dumps({"status": "sent", "contact": contact})
        responses = [
            f"Sent to {contact} on WhatsApp.",
            f"WhatsApp message delivered to {contact}.",
            f"Done. Messaged {contact}.",
        ]

        convos = [
            {"role": "system", "content": BOND_PROMPTS[bond]},
            {"role": "user", "content": user_msg},
            {"role": "assistant", "content": "", "tool_calls": [
                {"name": "whatsapp_send", "arguments": {"contact": contact, "message": msg}}
            ]},
            {"role": "tool", "name": "whatsapp_send", "content": result},
            {"role": "assistant", "content": random.choice(responses)},
        ]
        examples.append(make_line(convos, {
            "bond_stage": bond, "scenario_type": "task_request", "tools_used": ["whatsapp_send"]
        }))

    # ---------------------------------------------------------------
    # 9. send_notification (6 examples)
    # ---------------------------------------------------------------
    for i in range(6):
        bond = rand_bond()
        title = random.choice(NOTIFICATION_TITLES)
        notification_bodies = [
            "Your download of project-v2.zip is complete.",
            "Build #1247 passed all tests. Ready for deployment.",
            "Backup of /home completed successfully. 2.3 GB archived.",
            "New message from Sarah in the team channel.",
            "System update v2.4.1 is available. Restart to install.",
            "Disk usage on /dev/sda1 is at 92%. Consider cleanup.",
        ]
        body = notification_bodies[i]
        user_msgs = [
            f"Send me a notification: {title}",
            f"Notify me: {body}",
            f"Pop up a notification saying {title}",
            f"Show a desktop notification: {title}",
            f"Alert me: {body}",
            f"Push a notification: {title}",
        ]
        user_msg = user_msgs[i]

        result = json.dumps({"status": "sent", "notification_id": f"notif_{random.randint(100,999)}"})
        responses = [
            "Notification sent.",
            "Done. You should see it on your desktop.",
            f"Notification displayed: \"{title}\".",
        ]

        convos = [
            {"role": "system", "content": BOND_PROMPTS[bond]},
            {"role": "user", "content": user_msg},
            {"role": "assistant", "content": "", "tool_calls": [
                {"name": "send_notification", "arguments": {"title": title, "body": body}}
            ]},
            {"role": "tool", "name": "send_notification", "content": result},
            {"role": "assistant", "content": random.choice(responses)},
        ]
        examples.append(make_line(convos, {
            "bond_stage": bond, "scenario_type": "task_request", "tools_used": ["send_notification"]
        }))

    # ---------------------------------------------------------------
    # 10. Multi-tool: email_check + email_read (8 examples)
    # ---------------------------------------------------------------
    for i in range(8):
        bond = rand_bond()
        user_msgs = [
            "Check my email and read the most important one",
            "Any new emails? Show me the first one",
            "What's new in my inbox? Open the latest",
            "Check for emails and read anything from my boss",
            "New emails? Read the unread ones",
            "Check mail and show me the one about the project",
            "Any messages? Read the urgent ones",
            "Pull up my new emails and read the top one",
        ]
        user_msg = user_msgs[i]

        sender = random.choice(PEOPLE)
        sender_email = f"{sender.lower()}@{random.choice(EMAIL_DOMAINS)}"
        subject = random.choice(EMAIL_SUBJECTS)
        msg_id = f"msg_{random.randint(1000,9999)}"

        check_result = json.dumps({"new_count": random.randint(1, 5), "recent": [{"from": sender_email, "subject": subject, "id": msg_id}]})
        body = f"Hi, wanted to follow up on the {subject.lower()}. Let me know your thoughts. - {sender}"
        read_result = json.dumps({"id": msg_id, "from": sender_email, "subject": subject, "body": body, "date": rand_date_2026()})

        resp = f"You have new mail. Here's the latest from {sender}:\n\nSubject: {subject}\n\n{body}"

        convos = [
            {"role": "system", "content": BOND_PROMPTS[bond]},
            {"role": "user", "content": user_msg},
            {"role": "assistant", "content": "Checking your inbox.", "tool_calls": [
                {"name": "email_check", "arguments": {}},
            ]},
            {"role": "tool", "name": "email_check", "content": check_result},
            {"role": "assistant", "content": f"You have new mail. Let me pull up the latest.", "tool_calls": [
                {"name": "email_read", "arguments": {"message_id": msg_id}},
            ]},
            {"role": "tool", "name": "email_read", "content": read_result},
            {"role": "assistant", "content": resp},
        ]
        examples.append(make_line(convos, {
            "bond_stage": bond, "scenario_type": "multi_tool", "tools_used": ["email_check", "email_read"]
        }))

    # ---------------------------------------------------------------
    # 11. Multi-tool: email_list + email_read (6 examples)
    # ---------------------------------------------------------------
    for i in range(6):
        bond = rand_bond()
        folder = random.choice(["inbox", "sent", "drafts"])
        user_msgs = [
            f"Show me my {folder} and open the first email",
            f"List {folder} emails and read the newest one",
            f"What's in {folder}? Open the top message",
            f"Show {folder} and read the unread one",
            f"List my recent {folder} and open one from {random.choice(PEOPLE)}",
            f"Pull up {folder} and read the first message",
        ]
        user_msg = user_msgs[i]

        sender = random.choice(PEOPLE)
        sender_email = f"{sender.lower()}@{random.choice(EMAIL_DOMAINS)}"
        subject = random.choice(EMAIL_SUBJECTS)
        msg_id = f"msg_{random.randint(1000,9999)}"

        emails = [{"id": msg_id, "from": sender_email, "subject": subject, "date": rand_date_2026(), "read": False}]
        list_result = json.dumps({"folder": folder, "emails": emails, "total": len(emails)})

        body = f"Quick update on {subject.lower()}. Everything is on track. - {sender}"
        read_result = json.dumps({"id": msg_id, "from": sender_email, "subject": subject, "body": body, "date": rand_date_2026()})

        resp = f"From {sender}: {subject}\n\n{body}"

        convos = [
            {"role": "system", "content": BOND_PROMPTS[bond]},
            {"role": "user", "content": user_msg},
            {"role": "assistant", "content": "", "tool_calls": [
                {"name": "email_list", "arguments": {"folder": folder}},
            ]},
            {"role": "tool", "name": "email_list", "content": list_result},
            {"role": "assistant", "content": f"Found {len(emails)} email{'s' if len(emails) > 1 else ''}. Opening the top one.", "tool_calls": [
                {"name": "email_read", "arguments": {"message_id": msg_id}},
            ]},
            {"role": "tool", "name": "email_read", "content": read_result},
            {"role": "assistant", "content": resp},
        ]
        examples.append(make_line(convos, {
            "bond_stage": bond, "scenario_type": "multi_tool", "tools_used": ["email_list", "email_read"]
        }))

    # ---------------------------------------------------------------
    # 12. Multi-tool: email_read + email_reply (6 examples)
    # ---------------------------------------------------------------
    for i in range(6):
        bond = rand_bond()
        sender = random.choice(PEOPLE)
        sender_email = f"{sender.lower()}@{random.choice(EMAIL_DOMAINS)}"
        subject = random.choice(EMAIL_SUBJECTS)
        msg_id = f"msg_{random.randint(1000,9999)}"

        user_msgs = [
            f"Read {sender}'s email and reply saying I'll be there",
            f"Open the email from {sender} and reply with confirmation",
            f"Check {sender}'s message and send a quick reply",
            f"Read that email about {subject} and respond",
            f"Pull up {sender}'s email and reply that I agree",
            f"Open the latest from {sender} and tell them yes",
        ]
        user_msg = user_msgs[i]

        body = f"Hi, can you confirm your attendance for the {random.choice(MEETING_TOPICS)} on {rand_date_2026()}? - {sender}"
        read_result = json.dumps({"id": msg_id, "from": sender_email, "subject": subject, "body": body, "date": rand_date_2026()})

        reply_body = random.choice([
            f"Confirmed, I'll be there. Thanks {sender}.",
            f"Yes, count me in.",
            f"Got it, see you then.",
            f"Thanks for the heads up. I'll attend.",
        ])
        reply_result = json.dumps({"status": "sent", "message_id": f"msg_{random.randint(1000,9999)}", "in_reply_to": msg_id})

        resp = f"Done. I've read {sender}'s email and sent the reply."

        convos = [
            {"role": "system", "content": BOND_PROMPTS[bond]},
            {"role": "user", "content": user_msg},
            {"role": "assistant", "content": f"Let me read {sender}'s email first.", "tool_calls": [
                {"name": "email_read", "arguments": {"message_id": msg_id}},
            ]},
            {"role": "tool", "name": "email_read", "content": read_result},
            {"role": "assistant", "content": f"{sender} is asking about attendance. Sending your reply now.", "tool_calls": [
                {"name": "email_reply", "arguments": {"message_id": msg_id, "body": reply_body}},
            ]},
            {"role": "tool", "name": "email_reply", "content": reply_result},
            {"role": "assistant", "content": resp},
        ]
        examples.append(make_line(convos, {
            "bond_stage": bond, "scenario_type": "multi_tool", "tools_used": ["email_read", "email_reply"]
        }))

    # ---------------------------------------------------------------
    # 13. Error handling (10 examples)
    # ---------------------------------------------------------------
    error_scenarios = [
        ("Check my email", "email_check", {},
         '{"error": "auth_failed", "message": "Email authentication failed. Please re-enter credentials."}',
         "Your email credentials aren't working. You'll need to re-authenticate in Settings to reconnect."),
        ("Send an email to nobody@invalid", "email_send", {"to": "nobody@invalid", "subject": "Test", "body": "Hello"},
         '{"error": "invalid_recipient", "message": "Invalid email address: nobody@invalid"}',
         "That email address isn't valid. Double-check the address and try again."),
        ("Read message msg_0000", "email_read", {"message_id": "msg_0000"},
         '{"error": "not_found", "message": "Message not found"}',
         "I couldn't find that message. It may have been deleted or moved. Want me to search for it?"),
        ("Send a telegram to UnknownUser: hello", "telegram_send", {"contact": "UnknownUser", "message": "hello"},
         '{"error": "contact_not_found", "message": "Contact \'UnknownUser\' not found in Telegram"}',
         "I can't find a Telegram contact called UnknownUser. Check the name and try again."),
        ("Reply to the email from Jake", "email_reply", {"message_id": "msg_9999", "body": "Thanks!"},
         '{"error": "not_found", "message": "Original message not found"}',
         "I couldn't find the original email to reply to. It might have been archived or deleted."),
        ("Send a WhatsApp to Unknown: hi", "whatsapp_send", {"contact": "Unknown", "message": "hi"},
         '{"error": "not_configured", "message": "WhatsApp is not configured. Please link your account first."}',
         "WhatsApp isn't set up yet. You'll need to link your WhatsApp account in Settings before I can send messages."),
        ("Search emails for the contract from 2019", "email_search", {"query": "contract 2019"},
         '{"error": "timeout", "message": "Search timed out. Try a more specific query."}',
         "The search timed out -- 2019 is a broad range. Can you narrow it down? A sender name or more specific subject would help."),
        ("Send notification: test", "send_notification", {"title": "test", "body": "test"},
         '{"error": "notification_service_unavailable", "message": "Desktop notification service is not running"}',
         "The notification service isn't running right now. I'll queue this and send it when the service comes back."),
        ("Email the team: meeting cancelled", "email_send", {"to": "team@company.com", "subject": "Meeting Cancelled", "body": "The meeting is cancelled."},
         '{"error": "rate_limited", "message": "Too many emails sent in the last hour. Try again in 15 minutes."}',
         "You've hit the email rate limit. I'll queue this and send it in about 15 minutes."),
        ("Check my inbox", "email_list", {"folder": "inbox"},
         '{"error": "server_error", "message": "IMAP server returned error: connection reset"}',
         "The mail server connection dropped. This usually resolves on its own. I'll try again in a minute."),
    ]

    for user_msg, tool_name, args, error_result, response in error_scenarios:
        bond = rand_bond()
        convos = [
            {"role": "system", "content": BOND_PROMPTS[bond]},
            {"role": "user", "content": user_msg},
            {"role": "assistant", "content": "", "tool_calls": [
                {"name": tool_name, "arguments": args}
            ]},
            {"role": "tool", "name": tool_name, "content": error_result},
            {"role": "assistant", "content": response},
        ]
        examples.append(make_line(convos, {
            "bond_stage": bond, "scenario_type": "error_handling", "tools_used": [tool_name]
        }))

    # Pad to 100 if needed
    while len(examples) < 100:
        bond = rand_bond()
        contact = random.choice(TELEGRAM_CONTACTS)
        msg = f"Hey {contact}, just checking in. How's everything going?"
        user_msg = f"Send a telegram to {contact}: {msg}"
        result = json.dumps({"status": "sent", "chat_id": f"tg_{random.randint(100000,999999)}", "contact": contact})
        convos = [
            {"role": "system", "content": BOND_PROMPTS[bond]},
            {"role": "user", "content": user_msg},
            {"role": "assistant", "content": "", "tool_calls": [
                {"name": "telegram_send", "arguments": {"contact": contact, "message": msg}}
            ]},
            {"role": "tool", "name": "telegram_send", "content": result},
            {"role": "assistant", "content": f"Sent to {contact} on Telegram."},
        ]
        examples.append(make_line(convos, {
            "bond_stage": bond, "scenario_type": "task_request", "tools_used": ["telegram_send"]
        }))

    return examples[:100]


# ===================================================================
# Main
# ===================================================================

def validate_jsonl(lines, name):
    """Validate each line is valid JSON with required fields."""
    errors = 0
    tool_coverage = set()
    bond_counts = {"stranger": 0, "acquaintance": 0, "trusted": 0, "deep": 0}
    scenario_counts = {}
    multi_tool = 0
    error_handling = 0

    for i, line in enumerate(lines):
        try:
            obj = json.loads(line)
        except json.JSONDecodeError as e:
            print(f"  ERROR line {i+1}: invalid JSON: {e}")
            errors += 1
            continue

        if "conversations" not in obj:
            print(f"  ERROR line {i+1}: missing 'conversations'")
            errors += 1
            continue

        if "metadata" not in obj:
            print(f"  ERROR line {i+1}: missing 'metadata'")
            errors += 1
            continue

        convos = obj["conversations"]
        meta = obj["metadata"]

        # Check roles
        if convos[0]["role"] != "system":
            print(f"  ERROR line {i+1}: first message must be system")
            errors += 1
        if convos[1]["role"] != "user":
            print(f"  ERROR line {i+1}: second message must be user")
            errors += 1

        # Check tool_calls exist
        has_tool_call = any("tool_calls" in c for c in convos if c["role"] == "assistant")
        has_tool_result = any(c["role"] == "tool" for c in convos)
        if not has_tool_call:
            print(f"  ERROR line {i+1}: no tool_calls found")
            errors += 1
        if not has_tool_result:
            print(f"  ERROR line {i+1}: no tool result found")
            errors += 1

        # Track stats
        for tool in meta.get("tools_used", []):
            tool_coverage.add(tool)
        bond = meta.get("bond_stage", "")
        if bond in bond_counts:
            bond_counts[bond] += 1
        scenario = meta.get("scenario_type", "")
        scenario_counts[scenario] = scenario_counts.get(scenario, 0) + 1
        if scenario == "multi_tool":
            multi_tool += 1
        if scenario == "error_handling":
            error_handling += 1

    total = len(lines)
    print(f"\n  {name}: {total} examples, {errors} errors")
    print(f"  Tools covered: {sorted(tool_coverage)}")
    print(f"  Bond distribution: {bond_counts}")
    print(f"  Scenario types: {scenario_counts}")
    print(f"  Multi-tool: {multi_tool} ({multi_tool/total*100:.0f}%)")
    print(f"  Error handling: {error_handling} ({error_handling/total*100:.0f}%)")
    return errors


if __name__ == "__main__":
    print("Generating SCHEDULE examples...")
    schedule_lines = gen_schedule_examples()
    schedule_file = OUT_DIR / "batch_tools_07_schedule.jsonl"
    with open(schedule_file, "w", encoding="utf-8") as f:
        f.write("\n".join(schedule_lines) + "\n")
    print(f"  Wrote {len(schedule_lines)} lines to {schedule_file}")

    print("\nGenerating COMMUNICATE examples...")
    communicate_lines = gen_communicate_examples()
    communicate_file = OUT_DIR / "batch_tools_08_communicate.jsonl"
    with open(communicate_file, "w", encoding="utf-8") as f:
        f.write("\n".join(communicate_lines) + "\n")
    print(f"  Wrote {len(communicate_lines)} lines to {communicate_file}")

    print("\n--- Validation ---")
    e1 = validate_jsonl(schedule_lines, "SCHEDULE")
    e2 = validate_jsonl(communicate_lines, "COMMUNICATE")

    if e1 + e2 == 0:
        print("\nAll examples valid.")
    else:
        print(f"\nTotal errors: {e1 + e2}")
