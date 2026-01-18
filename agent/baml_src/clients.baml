// LLM Client Definitions for Trading Strategy

// Primary strategy inference client - using OpenRouter for flexibility
client StrategyLLM {
  provider openai-generic
  options {
    model "anthropic/claude-3.5-sonnet"
    base_url "https://openrouter.ai/api/v1"
    api_key env.OPENROUTER_API_KEY
  }
}

// Fast client for quick decisions
client FastLLM {
  provider openai-generic
  options {
    model "anthropic/claude-3-haiku"
    base_url "https://openrouter.ai/api/v1"
    api_key env.OPENROUTER_API_KEY
  }
}

// Fallback client using OpenAI directly
client FallbackLLM {
  provider openai
  options {
    model "gpt-4o-mini"
    api_key env.OPENAI_API_KEY
  }
}
