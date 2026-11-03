-- Suggested questions shown on the empty chat state.
insert into app_settings (key, value)
values (
    'suggested_questions',
    '[
      "What is the hostel fee per semester?",
      "How many books can an undergraduate borrow?",
      "When does the revaluation window open?",
      "Which bus route goes to the north campus?",
      "What are the mess timings?"
    ]'::jsonb
)
on conflict (key) do nothing;
