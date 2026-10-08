import { describe, expect, it, vi } from 'vitest'
import { act, fireEvent, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { FeedItem } from './FeedItem'
import { Feed } from '../types'

const feed: Feed = {
  id: 'f1',
  url: 'https://example.com/feed',
  title: 'Example',
  site_url: null,
  etag: null,
  last_modified: null,
  last_fetched_at: null,
  created_at: null,
  last_fetch_error: null,
  fetch_failing_since: null,
}

function renderItem(
  onDelete = vi.fn<() => Promise<void>>().mockResolvedValue(undefined),
  onSelect = vi.fn<() => void>(),
  feedOverride: Feed = feed,
) {
  render(
    <FeedItem
      feed={feedOverride}
      unreadCount={0}
      isSelected={false}
      onSelect={onSelect}
      onDelete={onDelete}
    />,
  )
  return { onDelete, onSelect }
}

describe('FeedItem', () => {
  it('deletes only on the second click', async () => {
    const user = userEvent.setup()
    const { onDelete, onSelect } = renderItem()

    await user.click(screen.getByTitle('削除'))
    expect(onDelete).not.toHaveBeenCalled()

    await user.click(screen.getByTitle('確認: もう一度クリックで削除'))
    expect(onDelete).toHaveBeenCalledTimes(1)
    expect(onSelect).not.toHaveBeenCalled()
  })

  it('shows a warning mark with the reason while the feed keeps failing', () => {
    // 失敗中のフィードだけ ⚠ を出し、title に理由を入れる (#245)。
    // 削除確認の `!` とは別物。
    const failing: Feed = {
      ...feed,
      last_fetch_error: 'HTTP 404',
      fetch_failing_since: 1757894400,
    }
    renderItem(
      vi.fn<() => Promise<void>>().mockResolvedValue(undefined),
      vi.fn<() => void>(),
      failing,
    )

    const mark = screen.getByText('⚠')
    const title = mark.getAttribute('title') ?? ''
    // title には理由と、失敗し始めた時刻の両方が入る (#245)。
    // 時刻の書式はロケール依存なので、理由以外の何かを含んでいることだけ見る。
    expect(title).toMatch(/HTTP 404/)
    expect(title).not.toBe('HTTP 404')
  })

  it('shows no warning mark while the feed is not failing', () => {
    // 失敗していないフィードには ⚠ を出さない (#245)。
    renderItem()
    expect(screen.queryByText('⚠')).toBeNull()
  })

  it('drops the confirmation after three seconds', () => {
    vi.useFakeTimers()
    try {
      const { onDelete } = renderItem()

      fireEvent.click(screen.getByTitle('削除'))
      act(() => {
        vi.advanceTimersByTime(3000)
      })
      fireEvent.click(screen.getByTitle('削除'))
      expect(onDelete).not.toHaveBeenCalled()
    } finally {
      vi.useRealTimers()
    }
  })
})
