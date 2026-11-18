<script lang="ts">
  import SocialClient from "$lib/social/SocialClient.svelte";
  import { NET } from "$lib/social/api";
</script>

<SocialClient platform={NET.forums} />
