<script setup>
import { computed } from 'vue'
import { useData } from 'vitepress'
import DefaultTheme from 'vitepress/theme'
import { groups } from '../navigation.mjs'
const { page } = useData()
const group = computed(() => groups.find(g => g.items.some(i => i.link === '/' + page.value.relativePath.replace(/\.md$/, '').replace(/\/index$/, '/')))?.text || 'Developer guide')
</script>

<template>
  <DefaultTheme.Layout>
    <template #nav-bar-title-before>
      <svg class="wordmark-symbol" viewBox="0 0 24 24" fill="none" aria-hidden="true">
        <path d="M3 4h10v13H3zM11 7h10v13H11z" stroke="currentColor" stroke-width="1.5" />
        <path d="M11 7h2v10h-2z" fill="currentColor" />
      </svg>
    </template>
    <template #nav-bar-title-after><span class="wordmark-edition">DOCS</span></template>
    <template #doc-before>
      <nav class="doc-breadcrumbs" aria-label="Breadcrumb">
        <a href="/docs/">Docs</a><span aria-hidden="true">/</span><span>{{ group }}</span>
        <span aria-hidden="true">/</span><span aria-current="page">{{ page.title }}</span>
      </nav>
    </template>
    <template #sidebar-nav-before>
      <div class="sidebar-edition">
        <span class="edition-label">Developer manual</span>
        <span class="edition-version">CLI 0.1.0 <span class="edition-platform">Linux / WSL2</span></span>
      </div>
    </template>
    <template #aside-outline-after>
      <div class="reference-note"><span>Execution model</span>Local LiteSVM<br />Prepared Anchor programs</div>
    </template>
  </DefaultTheme.Layout>
</template>
