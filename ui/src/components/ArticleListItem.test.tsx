import { describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { ArticleListItem } from './ArticleListItem'
import { Article } from '../types'
import { formatDate } from '../lib/date'

function article(overrides: Partial<Article> = {}): Article {
  return {
    id: 'a1',
    feed_id: 'f1',
    url: 'https://example.com/a1',
    title: 'Article 1',
    content: null,
    author: null,
    published_at: 1700000000,
    fetched_at: null,
    image_url: 'https://example.com/thumb.jpg',
    ...overrides,
  }
}

// このテストが捕まえる変異: 日付・サムネイルの表示条件の欠落と imgHidden リセットの欠落。
// published_at の fixture 値が具体的な epoch である理由は、formatDate と同じ関数で
// 期待値を組み立て、書式の重複定義を避けるため。
describe('ArticleListItem', () => {
  it('renders the title', () => {
    render(<ArticleListItem article={article()} isSelected={false} onClick={vi.fn()} />)
    expect(screen.getByText('Article 1')).toBeTruthy()
  })

  it('renders the formatted date when published_at exists', () => {
    render(<ArticleListItem article={article()} isSelected={false} onClick={vi.fn()} />)
    expect(screen.getByText(formatDate(1700000000))).toBeTruthy()
  })

  it('renders no date when published_at is null', () => {
    render(
      <ArticleListItem article={article({ published_at: null })} isSelected={false} onClick={vi.fn()} />,
    )
    expect(document.querySelector('.article-item-date')).toBeNull()
  })

  it('marks the row active only when selected', () => {
    const { unmount } = render(
      <ArticleListItem article={article()} isSelected={true} onClick={vi.fn()} />,
    )
    expect(document.querySelector('.article-item')?.className).toContain('article-item--active')
    unmount()

    render(<ArticleListItem article={article()} isSelected={false} onClick={vi.fn()} />)
    expect(document.querySelector('.article-item')?.className).not.toContain('article-item--active')
  })

  it('calls onClick when the row is clicked', async () => {
    const user = userEvent.setup()
    const onClick = vi.fn()
    render(<ArticleListItem article={article()} isSelected={false} onClick={onClick} />)
    await user.click(screen.getByText('Article 1'))
    expect(onClick).toHaveBeenCalledTimes(1)
  })

  it('shows the thumbnail when image_url exists', () => {
    render(<ArticleListItem article={article()} isSelected={false} onClick={vi.fn()} />)
    const img = document.querySelector('.article-item-thumbnail') as HTMLImageElement
    expect(img?.getAttribute('src')).toBe('https://example.com/thumb.jpg')
    expect(document.querySelector('.article-item')?.className).toContain('article-item--with-thumbnail')
  })

  it('shows no thumbnail when image_url is null', () => {
    render(
      <ArticleListItem article={article({ image_url: null })} isSelected={false} onClick={vi.fn()} />,
    )
    expect(document.querySelector('.article-item-thumbnail')).toBeNull()
    expect(document.querySelector('.article-item')?.className).not.toContain(
      'article-item--with-thumbnail',
    )
  })

  it('hides the thumbnail after an image error', () => {
    render(<ArticleListItem article={article()} isSelected={false} onClick={vi.fn()} />)
    fireEvent.error(document.querySelector('.article-item-thumbnail') as Element)
    expect(document.querySelector('.article-item-thumbnail')).toBeNull()
  })

  it('shows the thumbnail again when the article changes after an error', () => {
    const { rerender } = render(
      <ArticleListItem article={article()} isSelected={false} onClick={vi.fn()} />,
    )
    fireEvent.error(document.querySelector('.article-item-thumbnail') as Element)
    expect(document.querySelector('.article-item-thumbnail')).toBeNull()

    rerender(
      <ArticleListItem
        article={article({ id: 'a2', image_url: 'https://example.com/other.jpg' })}
        isSelected={false}
        onClick={vi.fn()}
      />,
    )
    expect(
      (document.querySelector('.article-item-thumbnail') as HTMLImageElement)?.getAttribute('src'),
    ).toBe('https://example.com/other.jpg')
  })
})
