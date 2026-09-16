import { createFileRoute } from '@tanstack/react-router'

import { Nav } from '#/components/site/nav'
import { CallToAction, Chef, FAQ, Footer, Hero, Relay, Tools, Turns } from '#/components/site/sections'

export const Route = createFileRoute('/')({ component: Home })

function Home() {
  return (
    <>
      <Nav />
      <main>
        <Hero />
        <div className="h-16 sm:h-24" />
        <Turns />
        <Relay />
        <Tools />
        <Chef />
        <FAQ />
        <CallToAction />
      </main>
      <Footer />
    </>
  )
}
