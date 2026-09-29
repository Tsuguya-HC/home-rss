import { beforeAll, describe, expect, it, vi } from 'vitest'
import { formatDate, formatDateTime } from './date'

// toLocaleString follows the runner's zone, so pin it for exact hour assertions.
beforeAll(() => {
  vi.stubEnv('TZ', 'UTC')
})

describe('formatDate', () => {
  it('formats a unix timestamp as month and day without the year', () => {
    expect(formatDate(1693526400)).toBe('9月1日')
  })

  it('does not zero-pad a single-digit day', () => {
    expect(formatDate(1735732800)).toBe('1月1日')
  })
})

describe('formatDateTime', () => {
  it('formats a unix timestamp with date, hour and minute', () => {
    expect(formatDateTime(1735732800)).toBe('2025年1月1日 12:00')
  })

  it('keeps midnight as 00:00', () => {
    expect(formatDateTime(1709251200)).toBe('2024年3月1日 00:00')
  })
})
