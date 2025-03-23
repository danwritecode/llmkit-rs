<template>
  <Html>
    <Head>
      <link rel="preconnect" href="https://fonts.googleapis.com">
      <link rel="preconnect" href="https://fonts.gstatic.com">
      <link href="https://fonts.googleapis.com/css2?family=JetBrains+Mono:wght@400;500;600;700;800&display=swap" rel="stylesheet">
    </Head>
    <Body class="bg-white dark:bg-neutral-900 transition-colors duration-300 font-mono antialiased">
      <NuxtLayout>
        <NuxtPage />
      </NuxtLayout>
    </Body>
  </Html>
</template>

<script setup lang="ts">
const colorMode = useColorMode()

// Server-side initial theme setup
useServerSeoMeta({
  // @ts-ignore
  script: [{
    innerHTML: import.meta.server ? `
      (function() {
        try {
          const stored = localStorage.getItem('nuxt-color-mode');
          const systemDark = window.matchMedia('(prefers-color-scheme: dark)').matches;
          const colorMode = stored === 'dark' ? 'dark' : stored === 'light' ? 'light' : systemDark ? 'dark' : 'light';
          document.documentElement.classList.toggle('dark', colorMode === 'dark');
        } catch(e) {}
      })()
    ` : ''
  }]
})

// Client-side watcher
if (import.meta.client) {
  watchEffect(() => {
    document.documentElement.classList.toggle('dark', colorMode.value === 'dark')
  })
}
</script>

<style>
.overflow-x-scroll::-webkit-scrollbar {
  height: 8px;
}

.overflow-x-scroll::-webkit-scrollbar-track {
  background: rgb(23, 23, 23); /* neutral-950 */
}

.overflow-x-scroll::-webkit-scrollbar-thumb {
  background: rgb(64, 64, 64); /* neutral-700 */
  border-radius: 4px;
}

.overflow-x-scroll::-webkit-scrollbar-thumb:hover {
  background: rgb(82, 82, 82); /* neutral-600 */
}

/* Firefox scrollbar styling */
.overflow-x-scroll {
  scrollbar-width: thin;
  scrollbar-color: rgb(64, 64, 64) rgb(23, 23, 23); /* thumb and track */
}
</style>
