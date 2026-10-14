import { defineConfig } from 'vitepress'
import { groups } from './navigation.mjs'
export default defineConfig({
  title: 'Doppelganger',
  description: 'Local Solana migration testing. Installation, prepared projects, policy, evidence and reproducible failures.',
  lang: 'en-US', cleanUrls: true,
  lastUpdated: false,
  srcExclude: ['README.md', 'VERIFICATION.md'],
  head: [['meta', { name: 'theme-color', content: '#a1442c' }]],
  themeConfig: {
    siteTitle: 'Doppelganger',
    nav: [{ text: 'Guide', link: '/docs/quickstart' }, { text: 'CLI reference', link: '/docs/cli' }, { text: '0.1.0', link: '/docs/verification' }],
    sidebar: { '/docs/': groups },
    outline: { level: [2, 3], label: 'On this page' },
    search: { provider: 'local' },
    docFooter: { prev: 'Previous', next: 'Next' },
    footer: { message: 'Configured evidence. Local execution. Reproducible results.', copyright: 'Doppelganger · MIT licensed' }
  }
})
