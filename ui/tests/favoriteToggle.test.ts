import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import { beginFavoriteToggle, endFavoriteToggle } from '../src/lib/favoriteToggle.ts'

// 2 本目の送信を弾かないと POST/DELETE が逆順に確定し、UI と DB が
// 食い違ったまま固定される (#152)。
describe('beginFavoriteToggle', () => {
  it('blocks a second send for the same article until the first finishes', () => {
    const inflight = new Set<string>()
    assert.equal(beginFavoriteToggle(inflight, 'a1'), true)
    assert.equal(beginFavoriteToggle(inflight, 'a1'), false)
    endFavoriteToggle(inflight, 'a1')
    assert.equal(beginFavoriteToggle(inflight, 'a1'), true)
  })

  it('tracks articles independently', () => {
    const inflight = new Set<string>()
    assert.equal(beginFavoriteToggle(inflight, 'a1'), true)
    assert.equal(beginFavoriteToggle(inflight, 'a2'), true)
  })

  it('endFavoriteToggle only releases the finished article', () => {
    const inflight = new Set<string>()
    beginFavoriteToggle(inflight, 'a1')
    beginFavoriteToggle(inflight, 'a2')
    endFavoriteToggle(inflight, 'a1')
    assert.equal(beginFavoriteToggle(inflight, 'a1'), true)
    assert.equal(beginFavoriteToggle(inflight, 'a2'), false)
  })
})
