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

// Agent state
let isRunning = false;
let cycleCount = 0;

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

/**
 * Main trading loop
 */
async function runTradingLoop(config) {
  isRunning = true;
  console.log("Starting DeFi trading agent...");
  console.log("Networks: " + config.networks.join(", "));
  console.log("Protocols: " + config.protocols.join(", "));
  console.log("Check interval: " + config.check_interval_ms + "ms");

  while (isRunning) {
    cycleCount++;
    console.log("\n=== Trading Cycle " + cycleCount + " ===");

    try {
      // Step 1: Gather pool data from all configured protocols
      console.log("Gathering pool data...");
      const pools = await gatherPoolData(config);
      console.log("Found " + pools.length + " pools");

      // Step 2: Get current market conditions
      console.log("Fetching market conditions...");
      const market = await getMarketConditions();
      console.log("ETH: $" + market.eth_price_usd + ", Sentiment: " + market.market_sentiment);

      // Step 3: Get current positions (simplified - would query wallet)
      const positions = await getCurrentPositions();

      // Step 4: Ask LLM for strategy recommendation
      console.log("Inferring strategy...");
      const action = await inferStrategy(pools, market, positions, config.risk);
      console.log("Strategy decision: " + action.action);

      // Step 5: Execute the recommended action
      await executeAction(action, config);

      // Wait before next iteration
      console.log("Waiting " + config.check_interval_ms + "ms before next cycle...");
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
async function gatherPoolData(config) {
  const pools = [];

  for (const network of config.networks) {
    for (const protocol of config.protocols) {
      try {
        const result = await invokeTool("query_subgraph", {
          protocol: protocol,
          network: network,
          query_type: "top_pools",
          params: { limit: 10 },
        });

        if (result.pools && Array.isArray(result.pools)) {
          for (const pool of result.pools) {
            pools.push({
              pool_id: pool.id,
              token0_symbol: pool.token0?.symbol || "???",
              token1_symbol: pool.token1?.symbol || "???",
              tvl_usd: parseFloat(pool.totalValueLockedUSD || "0"),
              volume_24h_usd: parseFloat(pool.volumeUSD || "0"),
              fee_tier: parseInt(pool.feeTier || "0"),
              token0_price: parseFloat(pool.token0Price || "0"),
              token1_price: parseFloat(pool.token1Price || "0"),
            });
          }
        }
      } catch (error) {
        console.error("Failed to query " + protocol + " on " + network + ":", error);
      }
    }
  }

  return pools;
}

/**
 * Get current market conditions
 */
async function getMarketConditions() {
  // In production, this would query price feeds and market data APIs
  // For now, using placeholder values
  return {
    eth_price_usd: 3500, // Would query from price oracle
    gas_price_gwei: 30, // Would query from network
    market_sentiment: "neutral",
  };
}

/**
 * Get current positions (simplified)
 */
async function getCurrentPositions() {
  // In production, this would query the wallet
  return [
    {
      token: "USDC",
      balance: "5000000000", // 5000 USDC (6 decimals)
      value_usd: 5000,
    },
  ];
}

/**
 * Call BAML function to infer trading strategy
 */
async function inferStrategy(pools, market, positions, risk) {
  // Call the BAML InferStrategy function with input wrapper
  const result = await InferStrategy({
    input: {
      pools: pools,
      market: market,
      positions: positions,
      risk_params: risk,
    }
  });

  return result;
}

/**
 * Execute a trading action
 */
async function executeAction(action, config) {
  switch (action.action) {
    case "query_pools":
      console.log("Need more data: " + action.reason);
      console.log("Will query " + action.protocol + " on " + action.network + " next cycle");
      break;

    case "swap":
      console.log("Swap opportunity identified:");
      console.log("  From: " + action.input_token);
      console.log("  To: " + action.output_token);
      console.log("  Amount: $" + action.amount_usd);
      console.log("  Network: " + action.network);
      console.log("  Confidence: " + (action.confidence * 100).toFixed(1) + "%");
      console.log("  Reasoning: " + action.reasoning);

      // Only execute if confidence is high enough
      if (action.confidence < 0.7) {
        console.log("Confidence too low, skipping trade");
        break;
      }

      // Get a quote first
      const quote = await getQuote(action);
      console.log("Quote received: expected output " + quote.output_amount);

      // Analyze the trade
      const analysis = await analyzeTrade(quote, action);
      console.log("Analysis: " + analysis.recommendation + " (" + analysis.risk_level + " risk)");

      if (analysis.recommendation === "execute") {
        // Prepare the swap - interceptors will enforce limits
        console.log("Preparing swap transaction...");
        const prepared = await prepareSwap(action, quote);
        console.log("Swap prepared: " + prepared.status);
        // Actual signing/submission happens in Rust after interceptor approval
      } else {
        console.log("Trade skipped: " + analysis.reasoning);
        if (analysis.concerns.length > 0) {
          console.log("Concerns: " + analysis.concerns.join(", "));
        }
      }
      break;

    case "wait":
      console.log("Waiting: " + action.reason);
      console.log("Suggested wait: " + action.duration_minutes + " minutes");
      break;
  }
}

/**
 * Get a swap quote from Odos
 */
async function getQuote(action) {
  // Convert USD amount to token amount (simplified - assumes USDC input)
  const amount_wei = Math.floor(action.amount_usd * 1e6).toString();

  const result = await invokeTool("odos_swap", {
    action: "quote",
    input_token: action.input_token,
    output_token: action.output_token,
    amount: amount_wei,
    network: action.network,
    slippage_percent: 0.5,
  });

  return result;
}

/**
 * Analyze a trade using BAML
 */
async function analyzeTrade(quote, action) {
  const result = await AnalyzeTrade({
    input: {
      quote_details: JSON.stringify(quote),
      pool_data: JSON.stringify({ network: action.network }),
      historical_context: "Recent market has been stable",
    }
  });

  return result;
}

/**
 * Prepare a swap transaction
 */
async function prepareSwap(action, quote) {
  const amount_wei = Math.floor(action.amount_usd * 1e6).toString();

  const result = await invokeTool("odos_swap", {
    action: "prepare_swap",
    input_token: action.input_token,
    output_token: action.output_token,
    amount: amount_wei,
    network: action.network,
    slippage_percent: 0.5,
  });

  return result;
}

/**
 * Sleep helper
 */
function sleep(ms) {
  return new Promise(function(resolve) { setTimeout(resolve, ms); });
}

/**
 * Stop the trading loop
 */
function stopTrading() {
  console.log("Stopping trading agent...");
  isRunning = false;
}

// Register functions globally
globalThis.runTradingLoop = runTradingLoop;
globalThis.stopTrading = stopTrading;
globalThis.TOKENS = TOKENS;

console.log("DeFi trading agent module loaded");
console.log("Call runTradingLoop(config) to start");
