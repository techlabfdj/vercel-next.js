// If it's already awaited, don't wrap it again
import { draftMode } from 'next/headers'

export default async function MyComponent() {
  await draftMode().enable()
}

export async function MyComponent2() {
  await draftMode().enable()
}