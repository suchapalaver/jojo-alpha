// Trading Strategy Functions
// These functions use LLMs to analyze market data and recommend actions

// ============================================================
// Data Types
// ============================================================

class PoolData {
  pool_id string @description("Pool contract address")
  token0_symbol string
  token1_symbol string
  tvl_usd float @description("Total value locked in USD")
  volume_24h_usd float @description("24-hour trading volume in USD")
  fee_tier int @description("Fee tier in basis points (e.g., 3000 = 0.3%)")
  token0_price float @description("Price of token0 in terms of token1")
  token1_price float @description("Price of token1 in terms of token0")
}

class MarketConditions {
  eth_price_usd float
  gas_price_gwei float
  market_sentiment string @description("bullish, bearish, or neutral")
}

class Position {
  token string @description("Token symbol")
  balance string @description("Balance in wei")
  value_usd float @description("Approximate USD value")
}

class RiskParameters {
  max_trade_usd float
  max_slippage_percent float
  preferred_networks string[] @description("Networks to trade on")
}

// ============================================================
// Action Types (Tool Calls)
// ============================================================

class QueryPoolsAction {
  action "query_pools" @description("Need more pool data before deciding")
  protocol_name string @description("Protocol: uniswap_v3")
  network_name string @description("Network: ethereum, arbitrum")
  reason string @description("Why more data is needed")
}

class SwapAction {
  action "swap" @description("Execute a swap")
  input_token string @description("Token address to sell")
  output_token string @description("Token address to buy")
  amount_usd float @description("Trade amount in USD")
  network string @description("Network to execute on")
  reasoning string @description("Detailed reasoning for this trade")
  confidence float @description("Confidence level 0-1")
}

class WaitAction {
  action "wait" @description("Hold position, don't trade")
  duration_minutes int @description("Suggested wait time")
  reason string @description("Why waiting is the best action")
}

// ============================================================
// Trade Analysis
// ============================================================

class TradeAnalysis {
  risk_level "low" | "medium" | "high"
  expected_profit_percent float @description("Expected profit/loss as percentage")
  recommendation "execute" | "skip" | "reduce_size"
  reasoning string @description("Detailed analysis")
  concerns string[] @description("List of concerns or risks")
}

// ============================================================
// Input types for functions
// ============================================================

class StrategyInput {
  pools PoolData[]
  market MarketConditions
  positions Position[]
  risk_params RiskParameters
}

class TradeAnalysisInput {
  quote_details string @description("JSON string of the swap quote")
  pool_data string @description("JSON string of pool metrics")
  historical_context string @description("Recent price movements and trends")
}

class MarketClassificationInput {
  eth_price_24h_change float
  total_volume_change float
  fear_greed_index int @description("0-100, where 0 is extreme fear")
}

// ============================================================
// Strategy Functions
// ============================================================

function InferStrategy(input: StrategyInput) -> SwapAction | QueryPoolsAction | WaitAction {
  client StrategyLLM
  prompt #"
    You are a DeFi trading strategist managing a USDC-denominated portfolio.
    Your goal is to identify profitable opportunities while staying within risk limits.

    ## Current Market Data

    ### Top Pools
    {% for pool in input.pools %}
    - {{ pool.token0_symbol }}/{{ pool.token1_symbol }} ({{ pool.fee_tier / 10000 }}% fee)
      TVL: ${{ pool.tvl_usd }}
      24h Volume: ${{ pool.volume_24h_usd }}
    {% endfor %}

    ### Market Conditions
    - ETH Price: ${{ input.market.eth_price_usd }}
    - Gas Price: {{ input.market.gas_price_gwei }} gwei
    - Sentiment: {{ input.market.market_sentiment }}

    ### Your Current Positions
    {% for pos in input.positions %}
    - {{ pos.token }}: ${{ pos.value_usd }}
    {% endfor %}

    ### Risk Parameters
    - Max trade size: ${{ input.risk_params.max_trade_usd }}
    - Max slippage: {{ input.risk_params.max_slippage_percent }}%
    - Allowed networks: {{ input.risk_params.preferred_networks }}

    ## Strategy Guidelines
    1. Volume/TVL ratio > 0.1 indicates an active, liquid pool
    2. Only trade if gas costs are < 5% of trade value
    3. Prefer larger pools (TVL > $1M) for better execution
    4. If market sentiment is bearish, prefer stablecoins
    5. Never trade more than the max_trade_usd limit
    6. If uncertain, recommend "wait" action

    ## Your Task
    Analyze the data and recommend ONE action:
    - "query_pools": If you need more data
    - "swap": If you see a clear opportunity
    - "wait": If conditions are unfavorable

    Be conservative. Only recommend swaps with solid reasoning.

    {{ ctx.output_format }}
  "#
}

function AnalyzeTrade(input: TradeAnalysisInput) -> TradeAnalysis {
  client StrategyLLM
  prompt #"
    You are a DeFi risk analyst. Analyze this potential trade and provide a recommendation.

    ## Swap Quote
    {{ input.quote_details }}

    ## Pool Metrics
    {{ input.pool_data }}

    ## Historical Context
    {{ input.historical_context }}

    ## Analysis Framework
    1. **Price Impact**: Is slippage acceptable?
    2. **Liquidity Depth**: Can the trade execute without moving the market?
    3. **Gas Efficiency**: Are gas costs reasonable relative to trade size?
    4. **Market Timing**: Is this a good entry/exit point?
    5. **Risk/Reward**: Does potential profit justify the risks?

    Provide your analysis with:
    - Risk level (low/medium/high)
    - Expected profit/loss percentage
    - Recommendation (execute/skip/reduce_size)
    - Detailed reasoning
    - List of specific concerns

    {{ ctx.output_format }}
  "#
}

function ClassifyMarketConditions(input: MarketClassificationInput) -> MarketConditions {
  client FastLLM
  prompt #"
    Based on the following metrics, classify the current market conditions:

    - ETH 24h price change: {{ input.eth_price_24h_change }}%
    - Total DeFi volume change: {{ input.total_volume_change }}%
    - Fear & Greed Index: {{ input.fear_greed_index }}

    Determine:
    1. Current ETH price in USD (estimate from change)
    2. Typical gas price for this market condition
    3. Overall market sentiment (bullish/bearish/neutral)

    Be objective and data-driven.

    {{ ctx.output_format }}
  "#
}
