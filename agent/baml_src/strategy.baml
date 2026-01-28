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
// Query Planning Types (Graph-Inference Bidirectional Flow)
// ============================================================

class QueryFilters {
  min_tvl_usd float @description("Minimum TVL in USD")
  min_volume_tvl_ratio float @description("Minimum volume/TVL ratio (e.g., 0.1)")
  token_pairs string[] @description("Specific token pairs to focus on (e.g., ['WETH/USDC'])")
  exclude_tokens string[] @description("Token addresses to exclude")
  min_volume_24h_usd float @description("Minimum 24h volume in USD")
  fee_tiers int[] @description("Fee tiers to include (e.g., [3000, 5000] for 0.3% and 0.5%)")
}

class QueryPlan {
  target_networks string[] @description("Networks to query (ethereum, arbitrum, etc.)")
  target_protocols string[] @description("Protocols to query (uniswap_v3, etc.)")
  data_filters QueryFilters @description("Filters to apply to query results")
  query_priority int @description("Priority: 1=urgent, 2=normal, 3=low")
  reasoning string @description("Why these queries are needed")
  expected_data_points int @description("How many pools/tokens to fetch")
}

class QueryPlanInput {
  current_positions Position[]
  recent_pools PoolData[] @description("Pools seen in recent cycles")
  market_conditions MarketConditions
  risk_params RiskParameters
  query_history string[] @description("Recent queries made (to avoid duplicates)")
  cycle_count int @description("Current trading cycle number")
}

class PartialDataInput {
  pools PoolData[] @description("Pools fetched so far (may be incomplete)")
  market MarketConditions
  positions Position[]
  risk_params RiskParameters
  query_plan QueryPlan @description("The query plan that generated this data")
  has_more_data bool @description("Whether more queries are pending")
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

// ============================================================
// Query Planning Functions (Graph-Inference Bidirectional Flow)
// ============================================================

function InferQueryPlan(input: QueryPlanInput) -> QueryPlan {
  client StrategyLLM
  prompt #"
    You are a DeFi data strategist. Your job is to decide what Graph queries to make
    based on current market context and trading goals.

    ## Current Context

    ### Positions
    {% for pos in input.current_positions %}
    - {{ pos.token }}: ${{ pos.value_usd }}
    {% endfor %}

    ### Recent Pool Data
    {% if input.recent_pools.length > 0 %}
    Recently seen pools:
    {% for pool in input.recent_pools %}
    - {{ pool.token0_symbol }}/{{ pool.token1_symbol }}: TVL ${{ pool.tvl_usd }}, Volume ${{ pool.volume_24h_usd }}
    {% endfor %}
    {% else %}
    No recent pool data available.
    {% endif %}

    ### Market Conditions
    - ETH Price: ${{ input.market_conditions.eth_price_usd }}
    - Sentiment: {{ input.market_conditions.market_sentiment }}
    - Gas: {{ input.market_conditions.gas_price_gwei }} gwei

    ### Risk Parameters
    - Max trade: ${{ input.risk_params.max_trade_usd }}
    - Max slippage: {{ input.risk_params.max_slippage_percent }}%
    - Preferred networks: {{ input.risk_params.preferred_networks }}

    ### Query History
    {% if input.query_history.length > 0 %}
    Recent queries: {{ input.query_history | join(", ") }}
    {% else %}
    No recent queries.
    {% endif %}

    ## Your Task

    Determine what Graph queries to make next. Consider:

    1. **Position Rebalancing**: If positions are imbalanced, query pools for rebalancing opportunities
    2. **Market Sentiment**: Bullish → query high-volume pools, Bearish → query stablecoin pools
    3. **Risk Limits**: If approaching limits, query safer pools (high TVL, stable pairs)
    4. **Data Gaps**: If recent_pools is empty or stale, query broadly
    5. **Efficiency**: Avoid querying the same data repeatedly (check query_history)

    ## Query Strategy Guidelines

    - **High Priority (1)**: Urgent rebalancing, risk management, or time-sensitive opportunities
    - **Normal Priority (2)**: Regular market scanning, position monitoring
    - **Low Priority (3)**: Exploratory queries, background data gathering

    - **TVL Filters**: Focus on pools with TVL > $1M for better execution
    - **Volume Filters**: Volume/TVL > 0.1 indicates active, liquid pools
    - **Token Pairs**: If positions contain specific tokens, query pools containing those tokens
    - **Network Selection**: Prefer networks in preferred_networks, but consider gas costs

    ## Output

    Provide a query plan that specifies:
    - Which networks and protocols to query
    - What filters to apply (TVL, volume, token pairs, etc.)
    - Priority level
    - Expected number of data points
    - Clear reasoning for why these queries are needed

    Be strategic: don't query everything, focus on what's needed for the next trading decision.

    {{ ctx.output_format }}
  "#
}

function InferFromPartialData(input: PartialDataInput) -> SwapAction | QueryPoolsAction | WaitAction {
  client StrategyLLM
  prompt #"
    You are analyzing partial Graph query results. Decide if you have enough data
    to make a trading decision, or if more queries are needed.

    ## Current Data

    ### Pools Retrieved
    {% if input.pools.length > 0 %}
    Found {{ input.pools.length }} pools:
    {% for pool in input.pools %}
    - {{ pool.token0_symbol }}/{{ pool.token1_symbol }}: TVL ${{ pool.tvl_usd }}, Volume ${{ pool.volume_24h_usd }}
    {% endfor %}
    {% else %}
    No pools retrieved yet.
    {% endif %}

    ### Query Plan Context
    - Target: {{ input.query_plan.target_networks | join(", ") }} on {{ input.query_plan.target_protocols | join(", ") }}
    - Filters: TVL > ${{ input.query_plan.data_filters.min_tvl_usd }}, Volume/TVL > {{ input.query_plan.data_filters.min_volume_tvl_ratio }}
    - Expected: {{ input.query_plan.expected_data_points }} pools
    - Reasoning: {{ input.query_plan.reasoning }}

    ### Market & Positions
    - ETH: ${{ input.market.eth_price_usd }}, Sentiment: {{ input.market.market_sentiment }}
    - Positions: {{ input.positions.length }} tokens
    - Risk limits: ${{ input.risk_params.max_trade_usd }} max trade

    ## Decision Framework

    1. **Enough Data?**
       - If pools.length < expected_data_points * 0.5 → likely need more queries
       - If all pools are below filters → query with relaxed filters
       - If found high-quality opportunities → can proceed to trade analysis

    2. **Data Quality**
       - Are pools meeting the query plan's filters?
       - Are there clear trading opportunities?
       - Is the data sufficient for risk assessment?

    3. **Next Action**
       - **query_pools**: If data is insufficient or quality is poor
       - **swap**: If clear opportunity with sufficient data
       - **wait**: If market conditions are unfavorable

    ## Guidelines

    - Be conservative: if uncertain, request more data
    - Don't wait for perfect data: if you have 5+ good pools, that may be enough
    - Consider query cost: don't request infinite queries
    - If query_plan priority was 1 (urgent), be more decisive with partial data

    {{ ctx.output_format }}
  "#
}
