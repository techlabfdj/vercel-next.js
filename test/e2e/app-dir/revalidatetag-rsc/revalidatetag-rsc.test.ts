import { nextTestSetup } from 'e2e-utils'
import { getRedboxHeader, retry } from 'next-test-utils'

describe('revalidateTag-rsc', () => {
  const { next, isNextDev, isNextDeploy } = nextTestSetup({
    files: __dirname,
  })

  it('should revalidate fetch cache if revalidateTag invoked via server action', async () => {
    const browser = await next.browser('/')
    const randomNumber = await browser.elementById('data').text()
    await browser.refresh()
    const randomNumber2 = await browser.elementById('data').text()
    expect(randomNumber).toEqual(randomNumber2)

    await browser.elementByCss('#submit-form').click()

    await retry(async () => {
      const randomNumber3 = await browser.elementById('data').text()
      expect(randomNumber3).not.toEqual(randomNumber)
    })
  })

  if (!isNextDeploy) {
    // skipped in deploy because it uses `next.cliOutput`
    it('should error if revalidateTag is called during render', async () => {
      const browser = await next.browser('/')
      await browser.elementByCss('#revalidate-via-page').click()

      if (isNextDev) {
        await retry(async () => {
          expect(await getRedboxHeader(browser)).toContain(
            'Route /revalidate_via_page used "revalidateTag data"'
          )
        })
      } else {
        await retry(async () => {
          expect(
            await browser.eval('document.documentElement.innerHTML')
          ).toContain('Application error: a server-side exception has occurred')
        })
      }

      expect(next.cliOutput).toContain(
        'Route /revalidate_via_page used "revalidateTag data"'
      )
    })
  }
})
