<script lang="ts">
  import Terminal from './Terminal.svelte'
</script>

<main>
  <Terminal />
</main>

<style>
  main {
    height: 100dvh;
    padding: env(safe-area-inset-top) env(safe-area-inset-right) env(safe-area-inset-bottom)
      env(safe-area-inset-left);
    box-sizing: border-box;
  }
</style>
