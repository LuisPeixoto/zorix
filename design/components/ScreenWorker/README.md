# Worker

One worker: how it is doing, how free it is, and what it did this week.

- Built from PageHeader, MetricTile, AutonomyDial, Field, Chip, Toggle, LogStream, Badge and Avatar.
- Four MetricTiles: tasks, on time, eval score and spend this week with a daily sparkline. The change reads in words and toned by whether it is good (more spend is `warn`).
- **Autonomy**: the dial, the weekly budget, the time for each task and the access patterns that always ask, whatever the level. A worker cannot be freer than who it reports to, and its budget cannot be larger than theirs. Tools that always ask still ask at every level.
- **Schedule** and *Plan the week* and *Write the report* appear for a virtual manager. The schedule is off until you turn it on and runs only while the app or `zorix serve` is open. **Reports to** and **Latest report** appear for a manager.
- **This week** is a LogStream of the worker's own lines.
