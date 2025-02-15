import type { Prompt, PromptExecutionResponse } from '../types/response/prompts'

export const usePrompts = () => {
  const prompts = ref<Prompt[]>([])
  const loading = ref(false)
  const error = ref<string | null>(null)

  const fetchPrompts = async () => {
    try {
      loading.value = true
      prompts.value = await $fetch<Prompt[]>('/api/v1/prompts')
    } catch (err) {
      console.error(err)
      error.value = 'Failed to fetch prompts'
    } finally {
      loading.value = false
    }
  }

  const createPrompt = async (promptData: { 
    key: string
    system: string
    user: string
    model_id: number 
    max_tokens: number
    temperature: number
    json_mode: boolean
  }) => {
    try {
      const newPrompt = await $fetch<Prompt>('/api/v1/prompts', {
        method: 'POST',
        body: {
          key: promptData.key,
          system: promptData.system,
          user: promptData.user,
          model_id: promptData.model_id,
          max_tokens: promptData.max_tokens,
          temperature: promptData.temperature,
          json_mode: promptData.json_mode
        }
      })
      prompts.value.push(newPrompt)
      return newPrompt
    } catch (err) {
      error.value = 'Failed to create prompt'
      throw err
    }
  }

  const updatePrompt = async (id: number, updates: { 
    key?: string
    system?: string
    user?: string
    model_id?: number 
    max_tokens: number
    temperature: number
    json_mode: boolean
  }) => {
    try {
      const updatedPrompt = await $fetch<Prompt>(`/api/v1/prompts/${id}`, {
        method: 'PUT',
        body: {
          key: updates.key,
          system: updates.system,
          user: updates.user,
          model_id: updates.model_id,
          max_tokens: updates.max_tokens,
          temperature: updates.temperature,
          json_mode: updates.json_mode
        }
      })
      const index = prompts.value.findIndex(p => p.id === id)
      if (index !== -1) prompts.value[index] = updatedPrompt
      return updatedPrompt
    } catch (err) {
      error.value = 'Failed to update prompt'
      throw err
    }
  }

  const deletePrompt = async (id: number) => {
    try {
      await $fetch(`/api/v1/prompts/${id}`, { method: 'DELETE' })
      prompts.value = prompts.value.filter(p => p.id !== id)
    } catch (err) {
      error.value = 'Failed to delete prompt'
      throw err
    }
  }

  const executePrompt = async (id: number, body: any) => {
    try {
      return await $fetch<PromptExecutionResponse>(`/api/v1/prompts/execute/${id}`, { 
        method: 'POST',
        body 
      })
    } catch (err) {
      error.value = 'Failed to execute prompt'
      throw err
    }
  }

  const executePromptStream = async (id: number, body: any) => {
    try {
      return await $fetch<string>(`/api/v1/prompts/execute/${id}/stream`, { 
        method: 'POST',
        body 
      })
    } catch (err) {
      error.value = 'Failed to execute prompt'
      throw err
    }
  }

  return {
    prompts,
    loading,
    error,
    fetchPrompts,
    createPrompt,
    updatePrompt,
    deletePrompt,
    executePrompt,
    executePromptStream
  }
}
