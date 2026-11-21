<script lang="ts">
  import type { Who } from "../../api.ts";
  import { projects } from "../../stores/projects.svelte.ts";
  import { Avatar } from "../../ui/index.ts";

  /** Who has a task or did something: an agent's or a team's face on a square, or a round avatar for you. */
  let { who, size = "sm" }: { who: Who | null; size?: "xs" | "sm" | "md" } = $props();
  const face = $derived(projects.face(who));
</script>

<Avatar human={face.human} emoji={face.emoji} color={face.color} {size} label={face.label} />
