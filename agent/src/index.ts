
// Host tool helper (agent-platform expects openToolSession for host tools).
// Note: This helper is duplicated across agent entrypoints; keep in sync.
async function invokeHostTool(toolName: string, args: any) {
  const token = (globalThis as any).__baml_invocation_token;
  if (!token) {
    throw new Error("Missing invocation token");
  }
  const session = await (globalThis as any).openToolSession(toolName, token);
  await session.send(args ?? {});
  let step = await session.continue();
  while (step && step.status === "streaming") {
    step = await session.continue();
  }
  await session.finish();
  if (step && step.status === "done") {
    return step.output;
  }
  if (step && step.status === "error") {
    throw new Error(step.error?.message || "Tool error");
  }
  return step;
}
const invokeTool = invokeHostTool;
(globalThis as any).invokeTool = invokeHostTool;

/**
 * DeFi Trading Agent
 *
 * This agent runs in the QuickJS sandbox and orchestrates trading by calling:
 * - BAML functions (LLM inference for strategy)
 * - Rust tools (The Graph queries, Odos swaps)
 *
 * SECURITY NOTE:
 * - This code runs sandboxed - no filesystem, no network, no private keys
 * - Can only interact with the outside world through registered tools
 * - All trades pass through interceptors before execution
 */

// Types matching BAML definitions
interface PoolData {
  pool_id: string;
  token0_symbol: string;
  token1_symbol: string;
  tvl_usd: number;
  volume_24h_usd: number;
  fee_tier: number;
  token0_price: number;
  token1_price: number;
}

interface MarketConditions {
  eth_price_usd: number;
  gas_price_gwei: number;
  market_sentiment: "bullish" | "bearish" | "neutral";
}

interface Position {
  token: string;
  balance: string;
  value_usd: number;
}

interface RiskParameters {
  max_trade_usd: number;
  max_slippage_percent: number;
  preferred_networks: string[];
}

interface TradingConfig {
  networks: string[];
  protocols: string[];
  check_interval_ms: number;
  risk: RiskParameters;
}

type TradingAction =
  | { action: "query_pools"; protocol: string; network: string; reason: string }
  | {
      action: "swap";
      input_token: string;
      output_token: string;
      amount_usd: number;
      network: string;
      reasoning: string;
      confidence: number;
    }
  | { action: "wait"; duration_minutes: number; reason: string };

interface TradeAnalysis {
  risk_level: "low" | "medium" | "high";
  expected_profit_percent: number;
  recommendation: "execute" | "skip" | "reduce_size";
  reasoning: string;
  concerns: string[];
}

// ============================================================
// Query Planning Types (Graph-Inference Bidirectional Flow)
// ============================================================

interface QueryFilters {
  min_tvl_usd?: number;
  min_volume_tvl_ratio?: number;
  token_pairs?: string[];
  exclude_tokens?: string[];
  min_volume_24h_usd?: number;
  fee_tiers?: number[];
}

interface QueryPlan {
  target_networks: string[];
  target_protocols: string[];
  data_filters: QueryFilters;
  query_priority: number;
  reasoning: string;
  expected_data_points: number;
}

interface TradingContext {
  cycleCount: number;
  positions: Position[];
  recentPools: PoolData[];
  queryHistory: string[];
  marketConditions: MarketConditions;
  lastQueryPlan?: QueryPlan;
  partialData: {
    pools: PoolData[];
    timestamp: number;
  };
}

// Agent state
let isRunning = false;
let cycleCount = 0;
let tradingContext: TradingContext | null = null;

// Well-known token addresses
const TOKENS = {
  ethereum: {
    USDC: "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
    WETH: "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2",
    USDT: "0xdac17f958d2ee523a2206206994597c13d831ec7",
  },
  arbitrum: {
    USDC: "0xaf88d065e77c8cc2239327c5edb3a432268e5831",
    WETH: "0x82af49447d8a07e3bd95bd0d56f35241523fbab1",
    USDT: "0xfd086bc7cd5c481dcc9c85ebe478a1c0b69fcbb9",
  },
};

// ============================================================
// Context Management Functions (Graph-Inference Bidirectional Flow)
// ============================================================

/**
 * Initialize trading context for a new session
 */
function initializeContext(config: TradingConfig): TradingContext {
  return {
    cycleCount: 0,
    positions: [],
    recentPools: [],
    queryHistory: [],
    marketConditions: {
      eth_price_usd: 3500,
      gas_price_gwei: 30,
      market_sentiment: "neutral",
    },
    partialData: {
      pools: [],
      timestamp: Date.now(),
    },
  };
}

/**
 * Update trading context with new data
 */
function updateContext(
  context: TradingContext,
  updates: {
    positions?: Position[];
    pools?: PoolData[];
    market?: MarketConditions;
    queryPlan?: QueryPlan;
    queryMade?: string;
  }
): TradingContext {
  return {
    ...context,
    cycleCount: context.cycleCount + 1,
    positions: updates.positions !== undefined ? updates.positions : context.positions,
    recentPools: updates.pools !== undefined ? updates.pools : context.recentPools,
    marketConditions: updates.market !== undefined ? updates.market : context.marketConditions,
    lastQueryPlan: updates.queryPlan !== undefined ? updates.queryPlan : context.lastQueryPlan,
    queryHistory: updates.queryMade
      ? [...context.queryHistory.slice(-9), updates.queryMade] // Keep last 10
      : context.queryHistory,
    partialData: updates.pools !== undefined
      ? {
          pools: updates.pools,
          timestamp: Date.now(),
        }
      : context.partialData,
  };
}

/**
 * Execute a query plan from the inference strategist
 */
async function executeQueryPlan(plan: QueryPlan): Promise<PoolData[]> {
  const pools: PoolData[] = [];

  try {
    const result = await invokeTool("defi/query_subgraph", {
      protocol: plan.target_protocols[0] || "uniswap_v3",
      network: plan.target_networks[0] || "ethereum",
      query_type: "query_plan",
      params: {
        query_plan: plan,
      },
    });

    // Extract pools from query plan results
    if (result.results && Array.isArray(result.results)) {
      for (const networkResult of result.results) {
        if (networkResult.data && networkResult.data.pools) {
          for (const pool of networkResult.data.pools) {
            pools.push({
              pool_id: pool.id,
              token0_symbol: (pool.token0 && pool.token0.symbol) || "???",
              token1_symbol: (pool.token1 && pool.token1.symbol) || "???",
              tvl_usd: parseFloat(pool.totalValueLockedUSD || "0"),
              volume_24h_usd: parseFloat(pool.volumeUSD || "0"),
              fee_tier: parseInt(pool.feeTier || "0"),
              token0_price: parseFloat(pool.token0Price || "0"),
              token1_price: parseFloat(pool.token1Price || "0"),
            });
          }
        }
      }
    }
  } catch (error) {
    console.error("Failed to execute query plan:", error);
  }

  return pools;
}

/**
 * Main trading loop (linear flow - legacy)
 */
async function runTradingLoop(config: TradingConfig): Promise<void> {
  isRunning = true;
  console.log("Starting DeFi trading agent (linear mode)...");
  console.log(`Networks: ${config.networks.join(", ")}`);
  console.log(`Protocols: ${config.protocols.join(", ")}`);
  console.log(`Check interval: ${config.check_interval_ms}ms`);

  while (isRunning) {
    cycleCount++;
    console.log(`\n=== Trading Cycle ${cycleCount} ===`);

    try {
      // Step 1: Gather pool data from all configured protocols
      console.log("Gathering pool data...");
      const pools = await gatherPoolData(config);
      console.log(`Found ${pools.length} pools`);

      // Step 2: Get current market conditions
      console.log("Fetching market conditions...");
      const market = await getMarketConditions();
      console.log(`ETH: $${market.eth_price_usd}, Sentiment: ${market.market_sentiment}`);

      // Step 3: Get current positions (simplified - would query wallet)
      const positions = await getCurrentPositions();

      // Step 4: Ask LLM for strategy recommendation
      console.log("Inferring strategy...");
      const action = await inferStrategy(pools, market, positions, config.risk);
      console.log(`Strategy decision: ${action.action}`);

      // Step 5: Execute the recommended action
      await executeAction(action, config);

      // Wait before next iteration
      console.log(`Waiting ${config.check_interval_ms}ms before next cycle...`);
      await sleep(config.check_interval_ms);
    } catch (error) {
      console.error("Trading loop error:", error);
      // Wait before retry
      await sleep(5000);
    }
  }

  console.log("Trading agent stopped.");
}

/**
 * Bidirectional trading loop with Graph-Inference interaction
 *
 * Flow:
 * 1. Inference Strategist plans queries (InferQueryPlan)
 * 2. Graph Orchestrator executes query plan
 * 3. Inference Strategist analyzes partial data (InferFromPartialData)
 * 4. Loop back to step 1 if more data needed, or execute action
 */
async function runBidirectionalTradingLoop(config: TradingConfig): Promise<void> {
  isRunning = true;
  console.log("Starting DeFi trading agent (Bidirectional Graph-Inference mode)...");
  console.log(`Networks: ${config.networks.join(", ")}`);
  console.log(`Protocols: ${config.protocols.join(", ")}`);
  console.log(`Check interval: ${config.check_interval_ms}ms`);

  // Initialize context
  let context = initializeContext(config);
  tradingContext = context;

  while (isRunning) {
    context = updateContext(context, {});
    console.log(`\n=== Trading Cycle ${context.cycleCount} (Bidirectional) ===`);

    try {
      // Step 1: Get current market conditions (lightweight)
      console.log("Fetching market conditions...");
      const market = await getMarketConditions();
      context = updateContext(context, { market });
      console.log(`ETH: $${market.eth_price_usd}, Sentiment: ${market.market_sentiment}`);

      // Step 2: Get current positions
      const positions = await getCurrentPositions();
      context = updateContext(context, { positions });
      console.log(`Positions: ${positions.length} tokens`);

      // Step 3: Expert 2 (Inference Strategist) plans queries
      console.log("Inference Strategist: Planning queries...");
      const queryPlan = await InferQueryPlan({
        input: {
          current_positions: context.positions,
          recent_pools: context.recentPools,
          market_conditions: context.marketConditions,
          risk_params: config.risk,
          query_history: context.queryHistory,
          cycle_count: context.cycleCount,
        },
      });

      console.log(`Query Plan: ${queryPlan.reasoning}`);
      console.log(`  Networks: ${queryPlan.target_networks.join(", ")}`);
      console.log(`  Protocols: ${queryPlan.target_protocols.join(", ")}`);
      console.log(`  Priority: ${queryPlan.query_priority}`);
      console.log(`  Expected pools: ${queryPlan.expected_data_points}`);

      context = updateContext(context, { queryPlan });

      // Step 4: Expert 1 (Graph Orchestrator) executes query plan
      console.log("Graph Orchestrator: Executing query plan...");
      const pools = await executeQueryPlan(queryPlan);
      console.log(`Retrieved ${pools.length} pools`);

      const queryKey = `${queryPlan.target_networks.join(",")}:${queryPlan.target_protocols.join(",")}`;
      context = updateContext(context, {
        pools,
        queryMade: queryKey,
      });

      // Step 5: Expert 2 analyzes partial data and decides next action
      console.log("Inference Strategist: Analyzing data...");
      const action = await InferFromPartialData({
        input: {
          pools: context.partialData.pools,
          market: context.marketConditions,
          positions: context.positions,
          risk_params: config.risk,
          query_plan: queryPlan,
          has_more_data: pools.length < queryPlan.expected_data_points,
        },
      });

      console.log(`Strategy decision: ${action.action}`);

      // Step 6: Handle the action
      if (action.action === "query_pools") {
        console.log(`Need more data: ${action.reason}`);
        console.log(`Will query ${action.protocol} on ${action.network} next cycle`);
        // Continue to next cycle (will plan new queries)
      } else {
        // Step 7: Execute the recommended action (swap or wait)
        await executeAction(action, config);
      }

      // Update context with recent pools for next cycle (keep top 20)
      const topPools = context.partialData.pools.slice(0, 20);
      context = updateContext(context, { pools: topPools });

      // Update global context reference
      tradingContext = context;

      // Wait before next iteration
      console.log(`Waiting ${config.check_interval_ms}ms before next cycle...`);
      await sleep(config.check_interval_ms);
    } catch (error) {
      console.error("Trading loop error:", error);
      // Wait before retry
      await sleep(5000);
    }
  }

  console.log("Trading agent stopped.");
}

/**
 * Gather pool data from The Graph subgraphs
 */
async function gatherPoolData(config: TradingConfig): Promise<PoolData[]> {
  const pools: PoolData[] = [];

  for (const network of config.networks) {
    for (const protocol of config.protocols) {
      try {
        const result = await invokeTool("defi/query_subgraph", {
          protocol,
          network,
          query_type: "top_pools",
          params: { limit: 10 },
        });

        if (result.pools && Array.isArray(result.pools)) {
          for (const pool of result.pools) {
            pools.push({
              pool_id: pool.id,
              token0_symbol: (pool.token0 && pool.token0.symbol) || "???",
              token1_symbol: (pool.token1 && pool.token1.symbol) || "???",
              tvl_usd: parseFloat(pool.totalValueLockedUSD || "0"),
              volume_24h_usd: parseFloat(pool.volumeUSD || "0"),
              fee_tier: parseInt(pool.feeTier || "0"),
              token0_price: parseFloat(pool.token0Price || "0"),
              token1_price: parseFloat(pool.token1Price || "0"),
            });
          }
        }
      } catch (error) {
        console.error(`Failed to query ${protocol} on ${network}:`, error);
      }
    }
  }

  return pools;
}

/**
 * Get current market conditions with real-time ETH price from Odos
 */
async function getMarketConditions(): Promise<MarketConditions> {
  let ethPriceUsd = 3500; // Fallback price

  try {
    // Get real-time ETH price via Odos (WETH -> USDC quote)
    ethPriceUsd = await getTokenPrice(TOKENS.ethereum.WETH, "ethereum");
    if (ethPriceUsd === 0) {
      ethPriceUsd = 3500; // Use fallback if price fetch failed
    }
  } catch (error) {
    console.error("Failed to get ETH price:", error);
  }

  // Gas price would ideally come from an RPC call
  // For now, using a reasonable estimate
  const gasPriceGwei = 30;

  // Sentiment could be derived from price trends
  // For now, using neutral
  const marketSentiment: "bullish" | "bearish" | "neutral" = "neutral";

  return {
    eth_price_usd: ethPriceUsd,
    gas_price_gwei: gasPriceGwei,
    market_sentiment: marketSentiment,
  };
}

/**
 * Get current positions by querying wallet balances with real-time prices
 */
async function getCurrentPositions(): Promise<Position[]> {
  const positions: Position[] = [];

  try {
    // Query wallet balances for all common tokens on Ethereum
    const result = await invokeTool("defi/wallet_balance", {
      action: "all_balances",
      network: "ethereum",
    });

    if (result.balances && Array.isArray(result.balances)) {
      // Collect token addresses for batch price lookup
      const tokenAddresses: string[] = [];
      for (const balance of result.balances) {
        if (balance.address) {
          tokenAddresses.push(balance.address);
        }
      }

      // Get real-time prices for all tokens
      const prices = await getTokenPrices(tokenAddresses, "ethereum");

      for (const balance of result.balances) {
        const amount = parseFloat(balance.balance_formatted);
        let valueUsd: number;

        // Use real price if available, otherwise fall back to estimate
        if (balance.address && prices[balance.address.toLowerCase()] !== undefined) {
          valueUsd = amount * prices[balance.address.toLowerCase()];
        } else {
          valueUsd = estimateUsdValue(balance.symbol, amount);
        }

        positions.push({
          token: balance.symbol,
          balance: balance.balance_raw,
          value_usd: valueUsd,
        });
      }
    }
  } catch (error) {
    console.error("Failed to query wallet balances:", error);
    // Return empty positions on error - agent will handle this gracefully
  }

  return positions;
}

// Cache for token prices to avoid repeated API calls within a cycle
let priceCache: Record<string, { price: number; timestamp: number }> = {};
const PRICE_CACHE_TTL_MS = 60000; // 1 minute cache

/**
 * Get real-time USD price for a token using Odos
 */
async function getTokenPrice(tokenAddress: string, network: string = "ethereum"): Promise<number> {
  const cacheKey = `${network}:${tokenAddress.toLowerCase()}`;
  const now = Date.now();

  // Check cache
  const cached = priceCache[cacheKey];
  if (cached && (now - cached.timestamp) < PRICE_CACHE_TTL_MS) {
    return cached.price;
  }

  try {
    const result = await invokeTool("defi/odos_swap", {
      action: "get_price",
      token: tokenAddress,
      network: network,
    });

    const price = result.price_usd || 0;

    // Cache the result
    priceCache[cacheKey] = { price, timestamp: now };

    return price;
  } catch (error) {
    console.error("Failed to get price for " + tokenAddress + ":", error);
    // Return cached price if available (even if stale), otherwise 0
    return cached ? cached.price : 0;
  }
}

/**
 * Get prices for multiple tokens efficiently
 */
async function getTokenPrices(tokenAddresses: string[], network: string = "ethereum"): Promise<Record<string, number>> {
  const prices: Record<string, number> = {};
  const now = Date.now();
  const tokensToFetch: string[] = [];

  // Check cache first
  for (const addr of tokenAddresses) {
    const cacheKey = `${network}:${addr.toLowerCase()}`;
    const cached = priceCache[cacheKey];
    if (cached && (now - cached.timestamp) < PRICE_CACHE_TTL_MS) {
      prices[addr.toLowerCase()] = cached.price;
    } else {
      tokensToFetch.push(addr);
    }
  }

  // Fetch missing prices
  if (tokensToFetch.length > 0) {
    try {
      const result = await invokeTool("defi/odos_swap", {
        action: "get_prices",
        tokens: tokensToFetch,
        network: network,
      });

      if (result.prices && Array.isArray(result.prices)) {
        for (const priceResult of result.prices) {
          if (priceResult.token && priceResult.price_usd !== undefined) {
            const addr = priceResult.token.toLowerCase();
            prices[addr] = priceResult.price_usd;
            priceCache[`${network}:${addr}`] = {
              price: priceResult.price_usd,
              timestamp: now
            };
          }
        }
      }
    } catch (error) {
      console.error("Failed to get batch prices:", error);
    }
  }

  return prices;
}

/**
 * Estimate USD value of a token balance
 * Uses real-time Odos prices with fallback to known values
 */
function estimateUsdValue(symbol: string, amount: number): number {
  // Stablecoins are always $1
  if (symbol === "USDC" || symbol === "USDT" || symbol === "DAI") {
    return amount * 1;
  }

  // For other tokens, use fallback prices
  // Real prices are fetched asynchronously in getCurrentPositions
  const fallbackPrices: Record<string, number> = {
    ETH: 3500,
    WETH: 3500,
    WBTC: 95000,
  };

  const price = fallbackPrices[symbol] || 0;
  return amount * price;
}

/**
 * Call BAML function to infer trading strategy
 */
async function inferStrategy(
  pools: PoolData[],
  market: MarketConditions,
  positions: Position[],
  risk: RiskParameters
): Promise<TradingAction> {
  // Call the BAML InferStrategy function with input wrapper
  const result = await InferStrategy({
    input: {
      pools,
      market,
      positions,
      risk_params: risk,
    }
  });

  return result as TradingAction;
}

/**
 * Execute a trading action
 */
async function executeAction(action: TradingAction, config: TradingConfig): Promise<void> {
  switch (action.action) {
    case "query_pools":
      console.log(`Need more data: ${action.reason}`);
      console.log(`Will query ${action.protocol} on ${action.network} next cycle`);
      break;

    case "swap":
      console.log(`Swap opportunity identified:`);
      console.log(`  From: ${action.input_token}`);
      console.log(`  To: ${action.output_token}`);
      console.log(`  Amount: $${action.amount_usd}`);
      console.log(`  Network: ${action.network}`);
      console.log(`  Confidence: ${(action.confidence * 100).toFixed(1)}%`);
      console.log(`  Reasoning: ${action.reasoning}`);

      // Only execute if confidence is high enough
      if (action.confidence < 0.7) {
        console.log("Confidence too low, skipping trade");
        break;
      }

      // Get a quote first
      const quote = await getQuote(action);
      console.log(`Quote received: expected output ${quote.output_amount}`);

      // Analyze the trade
      const analysis = await analyzeTrade(quote, action);
      console.log(`Analysis: ${analysis.recommendation} (${analysis.risk_level} risk)`);

      if (analysis.recommendation === "execute") {
        // Prepare the swap - interceptors will enforce limits
        console.log("Preparing swap transaction...");
        const prepared = await prepareSwap(action, quote);
        console.log(`Swap prepared: ${prepared.status}`);
        // Actual signing/submission happens in Rust after interceptor approval
      } else {
        console.log(`Trade skipped: ${analysis.reasoning}`);
        if (analysis.concerns.length > 0) {
          console.log(`Concerns: ${analysis.concerns.join(", ")}`);
        }
      }
      break;

    case "wait":
      console.log(`Waiting: ${action.reason}`);
      console.log(`Suggested wait: ${action.duration_minutes} minutes`);
      break;
  }
}

/**
 * Get a swap quote from Odos
 */
async function getQuote(action: { input_token: string; output_token: string; amount_usd: number; network: string }) {
  // Convert USD amount to token amount (simplified - assumes USDC input)
  const amount_wei = Math.floor(action.amount_usd * 1e6).toString();

  const result = await invokeTool("defi/odos_swap", {
    action: "quote",
    input_token: action.input_token,
    output_token: action.output_token,
    amount: amount_wei,
    amount_usd: action.amount_usd, // Pass USD value for spend limit interceptor
    network: action.network,
    slippage_percent: 0.5,
  });

  return result;
}

/**
 * Analyze a trade using BAML
 */
async function analyzeTrade(quote: any, action: any): Promise<TradeAnalysis> {
  const result = await AnalyzeTrade({
    input: {
      quote_details: JSON.stringify(quote),
      pool_data: JSON.stringify({ network: action.network }),
      historical_context: "Recent market has been stable",
    }
  });

  return result as TradeAnalysis;
}

/**
 * Prepare a swap transaction
 */
async function prepareSwap(action: any, quote: any) {
  const amount_wei = Math.floor(action.amount_usd * 1e6).toString();

  const result = await invokeTool("defi/odos_swap", {
    action: "prepare_swap",
    input_token: action.input_token,
    output_token: action.output_token,
    amount: amount_wei,
    amount_usd: action.amount_usd, // Pass USD value for spend limit interceptor
    network: action.network,
    slippage_percent: 0.5,
  });

  return result;
}

/**
 * Sleep helper
 */
function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/**
 * Stop the trading loop
 */
function stopTrading(): void {
  console.log("Stopping trading agent...");
  isRunning = false;
}

// Export to global scope for QuickJS bridge
declare global {
  function invokeTool(name: string, args: any): Promise<any>;
  function InferStrategy(args: any): Promise<any>;
  function AnalyzeTrade(args: any): Promise<any>;
  function InferQueryPlan(args: any): Promise<QueryPlan>;
  function InferFromPartialData(args: any): Promise<TradingAction>;
}

// Register functions globally
(globalThis as any).runTradingLoop = runTradingLoop;
(globalThis as any).runBidirectionalTradingLoop = runBidirectionalTradingLoop;
(globalThis as any).stopTrading = stopTrading;
(globalThis as any).TOKENS = TOKENS;

console.log("DeFi trading agent module loaded");
console.log("Call runTradingLoop(config) for linear mode");
console.log("Call runBidirectionalTradingLoop(config) for Graph-Inference bidirectional mode");
