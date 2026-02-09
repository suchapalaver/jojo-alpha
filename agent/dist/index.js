
// Host tool helper (agent-platform expects openToolSession for host tools)
async function invokeHostTool(toolName, args) {
  const token = globalThis.__baml_invocation_token;
  if (!token) {
    throw new Error("Missing invocation token");
  }
  const session = await globalThis.openToolSession(toolName, token);
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
    throw new Error((step.error && step.error.message) || "Tool error");
  }
  return step;
}
const invokeTool = invokeHostTool;
 globalThis.invokeTool = invokeHostTool;

(function() {
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
var isRunning = false;
var cycleCount = 0;

// Cache for token prices to avoid repeated API calls within a cycle
var priceCache = {};
var PRICE_CACHE_TTL_MS = 60000; // 1 minute cache

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
        const result = await invokeTool("defi/query_subgraph", {
          protocol: protocol,
          network: network,
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
        console.error("Failed to query " + protocol + " on " + network + ":", error);
      }
    }
  }

  return pools;
}

/**
 * Get real-time USD price for a token using Odos
 */
async function getTokenPrice(tokenAddress, network) {
  if (!network) network = "ethereum";
  var cacheKey = network + ":" + tokenAddress.toLowerCase();
  var now = Date.now();

  // Check cache
  var cached = priceCache[cacheKey];
  if (cached && (now - cached.timestamp) < PRICE_CACHE_TTL_MS) {
    return cached.price;
  }

  try {
    var result = await invokeTool("defi/odos_swap", {
      action: "get_price",
      token: tokenAddress,
      network: network,
    });

    var price = result.price_usd || 0;

    // Cache the result
    priceCache[cacheKey] = { price: price, timestamp: now };

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
async function getTokenPrices(tokenAddresses, network) {
  if (!network) network = "ethereum";
  var prices = {};
  var now = Date.now();
  var tokensToFetch = [];

  // Check cache first
  for (var i = 0; i < tokenAddresses.length; i++) {
    var addr = tokenAddresses[i];
    var cacheKey = network + ":" + addr.toLowerCase();
    var cached = priceCache[cacheKey];
    if (cached && (now - cached.timestamp) < PRICE_CACHE_TTL_MS) {
      prices[addr.toLowerCase()] = cached.price;
    } else {
      tokensToFetch.push(addr);
    }
  }

  // Fetch missing prices
  if (tokensToFetch.length > 0) {
    try {
      var result = await invokeTool("defi/odos_swap", {
        action: "get_prices",
        tokens: tokensToFetch,
        network: network,
      });

      if (result.prices && Array.isArray(result.prices)) {
        for (var j = 0; j < result.prices.length; j++) {
          var priceResult = result.prices[j];
          if (priceResult.token && priceResult.price_usd !== undefined) {
            var tokenAddr = priceResult.token.toLowerCase();
            prices[tokenAddr] = priceResult.price_usd;
            priceCache[network + ":" + tokenAddr] = {
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
 * Get current market conditions with real-time ETH price from Odos
 */
async function getMarketConditions() {
  var ethPriceUsd = 3500; // Fallback price

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
  var gasPriceGwei = 30;

  // Sentiment could be derived from price trends
  // For now, using neutral
  var marketSentiment = "neutral";

  return {
    eth_price_usd: ethPriceUsd,
    gas_price_gwei: gasPriceGwei,
    market_sentiment: marketSentiment,
  };
}

/**
 * Get current positions by querying wallet balances with real-time prices
 */
async function getCurrentPositions() {
  var positions = [];

  try {
    // Query wallet balances for all common tokens on Ethereum
    var result = await invokeTool("defi/wallet_balance", {
      action: "all_balances",
      network: "ethereum",
    });

    if (result.balances && Array.isArray(result.balances)) {
      // Collect token addresses for batch price lookup
      var tokenAddresses = [];
      for (var i = 0; i < result.balances.length; i++) {
        var balance = result.balances[i];
        if (balance.address) {
          tokenAddresses.push(balance.address);
        }
      }

      // Get real-time prices for all tokens
      var prices = await getTokenPrices(tokenAddresses, "ethereum");

      for (var j = 0; j < result.balances.length; j++) {
        var bal = result.balances[j];
        var amount = parseFloat(bal.balance_formatted);
        var valueUsd;

        // Use real price if available, otherwise fall back to estimate
        if (bal.address && prices[bal.address.toLowerCase()] !== undefined) {
          valueUsd = amount * prices[bal.address.toLowerCase()];
        } else {
          valueUsd = estimateUsdValue(bal.symbol, amount);
        }

        positions.push({
          token: bal.symbol,
          balance: bal.balance_raw,
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

/**
 * Estimate USD value of a token balance (fallback when Odos price unavailable)
 * Primary pricing now uses real-time Odos quotes via getTokenPrice/getTokenPrices
 */
function estimateUsdValue(symbol, amount) {
  // Stablecoins are always $1
  if (symbol === "USDC" || symbol === "USDT" || symbol === "DAI") {
    return amount * 1;
  }

  // Fallback prices for when Odos quote fails
  var fallbackPrices = {
    ETH: 3500,
    WETH: 3500,
    WBTC: 95000,
  };

  var price = fallbackPrices[symbol] || 0;
  return amount * price;
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
})();
