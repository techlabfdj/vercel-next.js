import { nextTestSetup } from 'e2e-utils'
import { assertHasRedbox, assertNoRedbox } from 'next-test-utils'

// Remove the location `()` part in every line of stack trace;
// Remove the leading spaces in every line of stack trace;
// Remove the trailing spaces in every line of stack trace;
function normalizeStackTrace(trace: string) {
  return trace
    .replace(/\(.*\)/g, '')
    .replace(/^\s+/gm, '')
    .trim()
}

async function getStackFramesContent(browser) {
  const stackFrameElements = await browser.elementsByCss(
    '[data-nextjs-call-stack-frame]'
  )
  const stackFramesContent = (
    await Promise.all(
      stackFrameElements.map(async (frame) => {
        const functionNameEl = await frame.$('[data-nextjs-frame-expanded]')
        const sourceEl = await frame.$('[data-has-source]')
        const functionName = functionNameEl
          ? await functionNameEl.innerText()
          : ''
        const source = sourceEl ? await sourceEl.innerText() : ''

        if (!functionName) {
          return ''
        }
        return functionName + ' @ ' + source
      })
    )
  )
    .filter(Boolean)
    .join('\n')

  return normalizeStackTrace(stackFramesContent)
}

describe('stitching errors', () => {
  const { next } = nextTestSetup({
    files: __dirname,
  })

  it('should log stitched error for browser uncaught errors', async () => {
    const browser = await next.browser('/browser/uncaught')

    await assertHasRedbox(browser)

    const stackFramesContent = await getStackFramesContent(browser)
    if (process.env.TURBOPACK) {
      expect(stackFramesContent).toMatchInlineSnapshot(`
        "useErrorHook @ app/browser/uncaught/page.js 
        Page @ app/browser/uncaught/page.js"
      `)
    } else {
      expect(stackFramesContent).toMatchInlineSnapshot(`
        "useThrowError @ app/browser/uncaught/page.js 
        useErrorHook @ app/browser/uncaught/page.js"
      `)
    }

    const logs = await browser.log()
    const errorLog = logs.find((log) => {
      return log.message.includes('Error: browser error')
    }).message

    if (process.env.TURBOPACK) {
      expect(normalizeStackTrace(errorLog)).toMatchInlineSnapshot(`
        "%o
        %s
        %s
        Error: browser error
        at useThrowError 
        at useErrorHook 
        at Page 
        at react-stack-bottom-frame 
        at renderWithHooks 
        at updateFunctionComponent 
        at beginWork 
        at runWithFiberInDEV 
        at performUnitOfWork 
        at workLoopSync 
        at renderRootSync 
        at performWorkOnRoot 
        at performWorkOnRootViaSchedulerTask 
        at MessagePort.performWorkUntilDeadline  The above error occurred in the <NotFoundErrorBoundary> component. React will try to recreate this component tree from scratch using the error boundary you provided, ReactDevOverlay."
      `)
    } else {
      expect(normalizeStackTrace(errorLog)).toMatchInlineSnapshot(`
        "%o
        %s
        %s
        Error: browser error
        at useThrowError 
        at useErrorHook 
        at Page 
        at react-stack-bottom-frame 
        at renderWithHooks 
        at updateFunctionComponent 
        at beginWork 
        at runWithFiberInDEV 
        at performUnitOfWork 
        at workLoopSync 
        at renderRootSync 
        at performWorkOnRoot 
        at performWorkOnRootViaSchedulerTask 
        at MessagePort.performWorkUntilDeadline  The above error occurred in the <NotFoundErrorBoundary> component. React will try to recreate this component tree from scratch using the error boundary you provided, ReactDevOverlay."
      `)
    }
  })

  it('should log stitched error for browser caught errors', async () => {
    const browser = await next.browser('/browser/caught')

    await assertNoRedbox(browser)

    const logs = await browser.log()
    const errorLog = logs.find((log) => {
      return log.message.includes('Error: browser error')
    }).message

    expect(normalizeStackTrace(errorLog)).toMatchInlineSnapshot(`
      "%o
      %s
      %s
      Error: browser error
      at useThrowError 
      at useErrorHook 
      at Thrower 
      at react-stack-bottom-frame 
      at renderWithHooks 
      at updateFunctionComponent 
      at beginWork 
      at runWithFiberInDEV 
      at performUnitOfWork 
      at workLoopSync 
      at renderRootSync 
      at performWorkOnRoot 
      at performWorkOnRootViaSchedulerTask 
      at MessagePort.performWorkUntilDeadline  The above error occurred in the <Thrower> component. React will try to recreate this component tree from scratch using the error boundary you provided, MyErrorBoundary."
    `)
  })

  it('should log stitched error for SSR errors', async () => {
    const browser = await next.browser('/ssr')

    await assertHasRedbox(browser)

    const stackFramesContent = await getStackFramesContent(browser)
    if (process.env.TURBOPACK) {
      expect(stackFramesContent).toMatchInlineSnapshot(`
        "useErrorHook @ app/ssr/page.js 
        Page @ app/ssr/page.js"
      `)
    } else {
      expect(stackFramesContent).toMatchInlineSnapshot(`
        "useThrowError @ app/ssr/page.js 
        useErrorHook @ app/ssr/page.js"
      `)
    }

    const logs = await browser.log()
    const errorLog = logs.find((log) => {
      return log.message.includes('Error: ssr error')
    }).message

    expect(normalizeStackTrace(errorLog)).toMatchInlineSnapshot(`
      "%o
      %s
      %s
      Error: ssr error
      at useThrowError 
      at useErrorHook 
      at Page 
      at react-stack-bottom-frame 
      at renderWithHooks 
      at updateFunctionComponent 
      at beginWork 
      at runWithFiberInDEV 
      at performUnitOfWork 
      at workLoopSync 
      at renderRootSync 
      at performWorkOnRoot 
      at performWorkOnRootViaSchedulerTask 
      at MessagePort.performWorkUntilDeadline  The above error occurred in the <NotFoundErrorBoundary> component. React will try to recreate this component tree from scratch using the error boundary you provided, ReactDevOverlay."
    `)
  })
})
