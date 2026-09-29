import { describe, expect, it, vi } from 'vitest'
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { ArticleDetail } from './ArticleDetail'
import { formatDateTime } from '../lib/date'
import { Article } from '../types'

// 全ての列が埋まった完全形を基準に、各テストが 1 列だけ欠損させて分岐を切り分ける
const baseArticle: Article = {
  id: 'a1',
  feed_id: 'f1',
  url: 'https://example.com/articles/1',
  title: 'Example article',
  content: '<p>Hello</p>',
  author: 'Example Author',
  published_at: 1727000000,
  fetched_at: 1727000100,
  image_url: null,
}

function metaSpanCount(container: HTMLElement): number {
  const meta = container.querySelector('.detail-meta')
  if (!meta) throw new Error('missing .detail-meta')
  return meta.querySelectorAll('span').length
}

describe('ArticleDetail', () => {
  // 捕まえる変異: 02-article-content (content の三項の反転)
  it('renders title, author, date, content, and a link to the source', () => {
    const { container } = render(<ArticleDetail article={baseArticle} onBack={vi.fn()} />)

    expect(screen.getByText('Example article')).not.toBeNull()
    expect(screen.getByText('Example Author')).not.toBeNull()
    expect(screen.getByText(formatDateTime(1727000000))).not.toBeNull()
    expect(screen.getByText('Hello')).not.toBeNull()
    expect(container.querySelector('.detail-content')).not.toBeNull()

    const links = screen.getAllByRole('link', { name: '元記事を開く ↗' })
    expect(links).toHaveLength(1)
    expect(links[0].getAttribute('href')).toBe('https://example.com/articles/1')
  })

  // 捕まえる変異: 01-article-author (author の条件の削除)
  it('omits the author span when author is null', () => {
    const { container } = render(
      <ArticleDetail article={{ ...baseArticle, author: null }} onBack={vi.fn()} />,
    )

    expect(screen.queryByText('Example Author')).toBeNull()
    expect(screen.getByText(formatDateTime(1727000000))).not.toBeNull()
    // 日付の span だけが残る。条件を外すと空の span が増えて 2 になる
    expect(metaSpanCount(container)).toBe(1)
  })

  // 捕まえる変異: 05-article-date (date の条件の削除)
  it('omits the date span when published_at is null', () => {
    const { container } = render(
      <ArticleDetail article={{ ...baseArticle, published_at: null }} onBack={vi.fn()} />,
    )

    expect(screen.getByText('Example Author')).not.toBeNull()
    // 著者の span だけが残る。条件を外すと空の span が増えて 2 になる
    expect(metaSpanCount(container)).toBe(1)
  })

  // 捕まえる変異: 02-article-content (content の三項の反転)
  it('shows the fallback with a source link when content is null', () => {
    const { container } = render(
      <ArticleDetail article={{ ...baseArticle, content: null }} onBack={vi.fn()} />,
    )

    expect(screen.getByText('本文がありません。')).not.toBeNull()
    const link = screen.getByRole('link', { name: '元記事を読む →' })
    expect(link.getAttribute('href')).toBe('https://example.com/articles/1')
    expect(container.querySelector('.detail-content')).toBeNull()
  })

  // 捕まえる変異: 02-article-content。空文字も欠損として扱う真偽判定を守る
  it('shows the fallback when content is an empty string', () => {
    render(<ArticleDetail article={{ ...baseArticle, content: '' }} onBack={vi.fn()} />)

    expect(screen.getByText('本文がありません。')).not.toBeNull()
  })

  // 捕まえる変異: 04-article-sanitize (DOMPurify.sanitize の削除)
  it('strips dangerous markup from content', () => {
    const { container } = render(
      <ArticleDetail
        article={{
          ...baseArticle,
          content: '<p>ok</p><script>window.__rss_probe = 1</script><img src="x" onerror="window.__rss_probe = 2">',
        }}
        onBack={vi.fn()}
      />,
    )

    expect(screen.getByText('ok')).not.toBeNull()
    expect(container.querySelector('script')).toBeNull()
    const img = container.querySelector('.detail-content img')
    if (!img) throw new Error('missing img in sanitized content')
    expect(img.getAttribute('onerror')).toBeNull()
  })

  // 捕まえる変異: 03-article-back (onBack の切り離し)
  it('calls onBack when the back button is clicked', async () => {
    const user = userEvent.setup()
    const onBack = vi.fn()
    render(<ArticleDetail article={baseArticle} onBack={onBack} />)

    await user.click(screen.getByTitle('戻る'))
    expect(onBack).toHaveBeenCalledTimes(1)
  })
})
