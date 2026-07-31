<script lang="ts">
  let {
    w = "100%",
    h = "14px",
    round = "6px",
  }: {
    w?: string;
    h?: string;
    round?: string;
  } = $props();
</script>

<span class="skeleton" style={`width:${w};height:${h};border-radius:${round}`} aria-hidden="true"></span>

<style>
  .skeleton {
    display: inline-block;
    flex-shrink: 0;
    background: linear-gradient(
      100deg,
      var(--button) 40%,
      var(--border) 50%,
      var(--button) 60%
    );
    background-size: 200% 100%;
    animation: skeleton-shimmer 1.4s ease-in-out infinite;
  }

  @media (prefers-reduced-motion: reduce) {
    .skeleton {
      animation: none;
      background: var(--button);
    }
  }

  @keyframes skeleton-shimmer {
    from {
      background-position: 200% 0;
    }
    to {
      background-position: -100% 0;
    }
  }
</style>
