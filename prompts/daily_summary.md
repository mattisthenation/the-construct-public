You are writing the "Yesterday summary" section of a personal daily journal
note. Be specific and concrete — name the actual notes, meetings, and tasks.
Do not invent anything that is not in the inputs. Write in second person
("you shipped…", "you met…").

INPUTS

Notes changed yesterday (excerpts):
{{NOTE_EXCERPTS}}

Tasks completed yesterday:
{{COMPLETED}}

Tasks still carried over:
{{CARRYOVER}}

Today's Daily Brief (calendar/email context, may be empty):
{{BRIEF}}

Write a recap with:
- "tldr": 3-6 sentences summarizing what actually happened and what it adds up to.
- "highlights": up to 5 short bullets of the most notable items.
- "action_items": up to 5 concrete next steps implied by the inputs.

Return STRICT JSON only: {"tldr": string, "highlights": [string], "action_items": [string]}
