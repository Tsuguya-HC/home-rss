import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import { isArticleVisible } from '../src/lib/articleFilter.ts'
import type { Article } from '../src/types.ts'

function article(overrides: Partial<Article> = {}): Article {
  return {
    id: 'a1',
    feed_id: 'f1',
    url: 'https://a.example/1',
    title: 'article',
    content: null,
    author: null,
    published_at: 1,
    fetched_at: 1,
    image_url: null,
    favorite: false,
    ...overrides,
  }
}

// お気に入りのみ表示中に解除した行が一覧に残ると、次に再取得するまで
// 星の空いた行が見え続ける (#152)。
describe('isArticleVisible', () => {
  it('hides an unfavorited article while favorites-only is on', () => {
    assert.equal(isArticleVisible(article({ favorite: false }), false, new Set(), true), false)
  })

  it('keeps a favorited article while favorites-only is on', () => {
    assert.equal(isArticleVisible(article({ favorite: true }), false, new Set(), true), true)
  })

  it('does not filter by favorite while favorites-only is off', () => {
    assert.equal(isArticleVisible(article({ favorite: false }), false, new Set(), false), true)
  })

  it('still hides read articles while unread-only is on', () => {
    assert.equal(
      isArticleVisible(article({ id: 'a1', favorite: true }), true, new Set(['a1']), false),
      false,
    )
    assert.equal(
      isArticleVisible(article({ id: 'a2', favorite: true }), true, new Set(['a1']), false),
      true,
    )
  })

  it('combines unread-only with favorites-only', () => {
    assert.equal(
      isArticleVisible(article({ id: 'a1', favorite: true }), true, new Set(['a1']), true),
      false,
    )
    assert.equal(
      isArticleVisible(article({ id: 'a2', favorite: true }), true, new Set(['a1']), true),
      true,
    )
  })
})
