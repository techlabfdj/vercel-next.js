import type { NextConfigComplete } from '../config-shared'

import '../require-hook'
import '../node-environment'

import {
  buildAppStaticPaths,
  buildStaticPaths,
  reduceAppConfig,
} from '../../build/utils'
import { collectSegments } from '../../build/app-segments/collect-app-segments'
import type { PartialStaticPathsResult } from '../../build/utils'
import { loadComponents } from '../load-components'
import { setHttpClientAndAgentOptions } from '../setup-http-agent-env'
import type { IncrementalCache } from '../lib/incremental-cache'
import { isAppPageRouteModule } from '../route-modules/checks'
import {
  checkIsRoutePPREnabled,
  type ExperimentalPPRConfig,
} from '../lib/experimental/ppr'

type RuntimeConfig = {
  pprConfig: ExperimentalPPRConfig | undefined
  configFileName: string
  publicRuntimeConfig: { [key: string]: any }
  serverRuntimeConfig: { [key: string]: any }
  dynamicIO: boolean
}

// we call getStaticPaths in a separate process to ensure
// side-effects aren't relied on in dev that will break
// during a production build
export async function loadStaticPaths({
  dir,
  distDir,
  pathname,
  config,
  httpAgentOptions,
  locales,
  defaultLocale,
  isAppPath,
  page,
  isrFlushToDisk,
  fetchCacheKeyPrefix,
  maxMemoryCacheSize,
  requestHeaders,
  cacheHandler,
  nextConfigOutput,
  isAppPPRFallbacksEnabled,
  buildId,
}: {
  dir: string
  distDir: string
  pathname: string
  config: RuntimeConfig
  httpAgentOptions: NextConfigComplete['httpAgentOptions']
  locales?: string[]
  defaultLocale?: string
  isAppPath: boolean
  page: string
  isrFlushToDisk?: boolean
  fetchCacheKeyPrefix?: string
  maxMemoryCacheSize?: number
  requestHeaders: IncrementalCache['requestHeaders']
  cacheHandler?: string
  nextConfigOutput: 'standalone' | 'export' | undefined
  isAppPPRFallbacksEnabled: boolean | undefined
  buildId: string
}): Promise<PartialStaticPathsResult> {
  // update work memory runtime-config
  require('../../shared/lib/runtime-config.external').setConfig(config)
  setHttpClientAndAgentOptions({
    httpAgentOptions,
  })

  const components = await loadComponents({
    distDir,
    // In `pages/`, the page is the same as the pathname.
    page: page || pathname,
    isAppPath,
  })

  if (!components.getStaticPaths && !isAppPath) {
    // we shouldn't get to this point since the worker should
    // only be called for SSG pages with getStaticPaths
    throw new Error(
      `Invariant: failed to load page with getStaticPaths for ${pathname}`
    )
  }

  if (isAppPath) {
    const segments = await collectSegments(components)

    const isRoutePPREnabled =
      isAppPageRouteModule(components.routeModule) &&
      checkIsRoutePPREnabled(config.pprConfig, reduceAppConfig(segments))

    return await buildAppStaticPaths({
      dir,
      page: pathname,
      dynamicIO: config.dynamicIO,
      segments,
      configFileName: config.configFileName,
      distDir,
      requestHeaders,
      cacheHandler,
      isrFlushToDisk,
      fetchCacheKeyPrefix,
      maxMemoryCacheSize,
      ComponentMod: components.ComponentMod,
      nextConfigOutput,
      isRoutePPREnabled,
      isAppPPRFallbacksEnabled,
      buildId,
    })
  }

  return await buildStaticPaths({
    page: pathname,
    getStaticPaths: components.getStaticPaths,
    configFileName: config.configFileName,
    locales,
    defaultLocale,
  })
}
