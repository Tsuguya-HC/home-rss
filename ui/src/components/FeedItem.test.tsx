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

const failingFeed: Feed = {
  ...feed,
  id: 'f2',
  last_fetch_error: 'HTTP 404',
  fetch_failing_since: 1_757_894_400,
} as Feed

function renderItem(
  item: Feed = feed,
  onDelete = vi.fn<() => Promise<void>>().mockResolvedValue(undefined),
  onSelect = vi.fn<() => void>(),
) {
  render(
    <FeedItem
      feed={item}
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

  it('shows a warning for a failing feed', () => {
    // #245: 失敗中のフィードだけ警告マーク（⚠）を出す。削除確認の `!` とは別物。
    // title には理由を入れる。
    renderItem(failingFeed)

    expect(screen.getByText('⚠')).toBeTruthy()
    expect(screen.getByTitle(/HTTP 404/)).toBeTruthy()
  })

  it('shows a warning without a start time when the record has no timestamp', () => {
    // fetch_failing_since が NULL でもマークと理由は出す。DB の列は NULL 可なので、
    // 直書きされた行で時刻が無い場合も壊さず表示する。
    renderItem({ ...failingFeed, fetch_failing_since: null })

    expect(screen.getByText('⚠')).toBeTruthy()
    expect(screen.getByTitle(/HTTP 404/)).toBeTruthy()
  })

  it('shows no warning for a healthy feed', () => {
    renderItem()

    expect(screen.queryByText('⚠')).toBeNull()
  })
})
