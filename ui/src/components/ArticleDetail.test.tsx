import { describe, expect, it, vi } from 'vitest'
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { ArticleDetail } from './ArticleDetail'
import { formatDateTime } from '../lib/date'
import { Article } from '../types'

const article: Article = {
  id: 'a1',
  feed_id: 'f1',
  url: 'https://example.com/articles/1',
  title: 'Example article',
  content: '<p>Hello</p>',
  author: 'Jane Doe',
  published_at: 1700000000,
  fetched_at: null,
  image_url: null,
}

function renderDetail(override: Partial<Article> = {}, onBack = vi.fn<() => void>()) {
  const view = render(<ArticleDetail article={{ ...article, ...override }} onBack={onBack} />)
  return { onBack, ...view }
}

describe('ArticleDetail', () => {
  it('renders the title, author, date, content and the link to the original', () => {
    renderDetail()

    expect(screen.getByText('Example article')).toBeTruthy()
    expect(screen.getByText('Jane Doe')).toBeTruthy()
    expect(screen.getByText(formatDateTime(1700000000))).toBeTruthy()
    expect(screen.getByText('Hello')).toBeTruthy()
    const link = screen.getByText('元記事を開く ↗').closest('a')
    expect(link?.getAttribute('href')).toBe('https://example.com/articles/1')
  })

  // フォールバック表示を足すと meta に余分な文言が残る。
  it('hides the author when it is missing', () => {
    const { container } = renderDetail({ author: null })

    expect(container.querySelector('.detail-meta')?.textContent).toBe(formatDateTime(1700000000))
  })

  it('hides the date when published_at is missing', () => {
    const { container } = renderDetail({ published_at: null })

    expect(container.querySelector('.detail-meta')?.textContent).toBe('Jane Doe')
  })

  it('shows the fallback when the content is missing', () => {
    const { container } = render(
      <ArticleDetail article={{ ...article, content: null }} onBack={vi.fn<() => void>()} />,
    )

    expect(screen.getByText('本文がありません。')).toBeTruthy()
    expect(container.querySelector('.detail-content')).toBeNull()
  })

  // sanitize を外すと script 要素が残る。
  it('strips scripts from the content', () => {
    const { container } = render(
      <ArticleDetail
        article={{ ...article, content: '<script>alert(1)</script><p>safe</p>' }}
        onBack={vi.fn<() => void>()}
      />,
    )

    expect(container.querySelector('script')).toBeNull()
    expect(screen.getByText('safe')).toBeTruthy()
  })

  it('calls onBack when the back button is clicked', async () => {
    const user = userEvent.setup()
    const { onBack } = renderDetail()

    await user.click(screen.getByTitle('戻る'))

    expect(onBack).toHaveBeenCalledTimes(1)
  })
})
