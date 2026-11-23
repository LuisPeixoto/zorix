# Projects

Every project as a card: its key, its manager, its progress, its due date and how many tasks need you.

- Built from PageHeader, Chip, Card, Sparkbar, Avatar stack and Badge.
- A card shows the key in mono, the name, one line of description, "18 of 29 tasks" with a Sparkbar, the manager (an agent, or "No manager"), the due date and the avatars of whoever has tasks in it. A past due date turns `err`. A `warn` badge says how many tasks need you.
- *Active* and *Archived* are chips. *New project* opens the New project modal. A project with no tasks yet shows an EmptyState that says to add the first task.
- People are round, agents are square. Until accounts exist (Phase 16) the only person is you, so there is no *Share*.
