import type {
  CacheNodeSeedData,
  FlightDataPath,
  FlightRouterState,
  FlightSegmentPath,
  Segment,
} from '../server/app-render/types'

export function getFlightDataPartsFromPath(flightDataPath: FlightDataPath): {
  segmentPath: FlightSegmentPath[]
  segment: Segment
  treePatch: FlightRouterState
  seedData: CacheNodeSeedData | null
  head: React.ReactNode | null
} {
  const [treePatch, seedData, head] = flightDataPath.slice(-3)
  const remainingPath = flightDataPath.slice(0, -3)
  // if the `FlightDataPath` corresponds with the root, there'll be no segment, in which case
  // we default to ''. But if there is, we pop it off, and the remaining path is the `segmentPath`.
  // This is because `FlightDataPath` is a repeating tuple of segment paths leading up to the
  // leaf segment data.
  const segment = remainingPath.pop() || ''
  const segmentPath = remainingPath

  return {
    segmentPath,
    segment,
    treePatch,
    seedData,
    head,
  }
}

export function isRootFlightDataPath(flightDataPath: FlightDataPath): boolean {
  return flightDataPath.length === 3
}

export function isLastFlightDataPathEntry(
  flightDataPath: FlightDataPath
): boolean {
  return flightDataPath.length === 5
}

export function getNextFlightSegmentPath(
  flightSegmentPath: FlightSegmentPath
): FlightSegmentPath {
  // Since `FlightSegmentPath` is a repeated tuple of `Segment` and `ParallelRouteKey`, we slice off two items
  // to get the next segment path.
  return flightSegmentPath.slice(2)
}
