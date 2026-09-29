import { describe, expect, it, vi } from 'vitest'
import { act, fireEvent, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { AddFeedModal } from './AddFeedModal'

const FEED_URL = 'https://example.com/feed.xml'

function renderModal(
  onAdd = vi.fn<(url: string) => Promise<void>>().mockResolvedValue(undefined),
  onClose = vi.fn<() => void>(),
) {
  const view = render(<AddFeedModal onAdd={onAdd} onClose={onClose} />)
  return { onAdd, onClose, ...view }
}

function submitWith(url: string) {
  fireEvent.change(screen.getByPlaceholderText('https://example.com/feed.xml'), {
    target: { value: url },
  })
  fireEvent.submit(screen.getByText('フィード追加').closest('.modal')!.querySelector('form')!)
}

describe('AddFeedModal', () => {
  it('calls onAdd with the entered url on submit', () => {
    const { onAdd } = renderModal()

    submitWith(FEED_URL)

    expect(onAdd).toHaveBeenCalledTimes(1)
    expect(onAdd).toHaveBeenCalledWith(FEED_URL)
  })

  it('does not call onAdd for a blank input', () => {
    const { onAdd } = renderModal()

    submitWith('   ')

    expect(onAdd).not.toHaveBeenCalled()
  })

  // loading 中の disabled を外すとこのテストが落ちる。
  it('disables the submit button until onAdd resolves', async () => {
    let resolve!: () => void
    const onAdd = vi.fn<() => Promise<void>>(
      () =>
        new Promise<void>((r) => {
          resolve = r
        }),
    )
    renderModal(onAdd)

    submitWith(FEED_URL)
    expect((screen.getByRole('button', { name: '追加中...' }) as HTMLButtonElement).disabled).toBe(
      true,
    )

    await act(async () => {
      resolve()
    })
    expect((screen.getByRole('button', { name: '追加' }) as HTMLButtonElement).disabled).toBe(false)
  })

  it('calls onClose when the cancel button is clicked', async () => {
    const user = userEvent.setup()
    const { onClose } = renderModal()

    await user.click(screen.getByRole('button', { name: 'キャンセル' }))

    expect(onClose).toHaveBeenCalledTimes(1)
  })

  it('calls onClose on overlay click but not on modal click', () => {
    const { container, onClose } = renderModal()

    fireEvent.click(screen.getByPlaceholderText('https://example.com/feed.xml'))
    expect(onClose).not.toHaveBeenCalled()

    fireEvent.click(container.querySelector('.modal-overlay')!)
    expect(onClose).toHaveBeenCalledTimes(1)
  })
})
