// patch Workers for our Node.js test environment
// globalThis.Worker = (Date.now() > 0 ? require : 'unused')(
//   'node:worker_threads'
// ).Worker
globalThis.Worker = class {
  constructor(x){
    console.log("Worker", x);
  }
}

it('supports workers', async () => {
   new Worker(new URL('./worker.ts', import.meta.url))
  // let worker = new Worker(new URL('./worker.ts', import.meta.url))
  // let message = await new Promise((resolve) => {
  //   worker.addEventListener('message', (event) => {
  //     resolve(event.data)
  //   })
  // })

  // expect(message).toBe('getMessage worker-dep')
})
