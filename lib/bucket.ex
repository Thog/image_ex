defmodule ImageEx.Bucket do
    use GenServer
    require Logger
  
    @key Application.get_env(:image_ex, :puush_key)
    def start_link() do
      GenServer.start_link(__MODULE__, File.read!("#{Application.get_env(:image_ex, :path)}/app.json") |> Poison.decode!, [name: __MODULE__])
    end
  
    def init(state) do
      #schedule_caching()
      {:ok, state}
    end
  
    def do_caching(state) do
      res = File.ls!("#{Application.get_env(:image_ex, :path)}/bucket/")
        |> Enum.reduce(state, fn (img, %{"cached" => cache}=state) ->
          case Puush.up(@key, "#{Application.get_env(:image_ex, :path)}/bucket/#{img}") do
            {:ok, url} ->
              res = state |> put_in(["cached"], Map.put(cache, img, url))
              File.rm("#{Application.get_env(:image_ex, :path)}/bucket/#{img}")
              res
            {:error, error} ->
              IO.puts("Error trying to upload #{img}: #{error || "Unknown"}")
              state
          end
      end)
      File.write!("#{Application.get_env(:image_ex, :path)}/app.json", res |> Poison.encode!)
      res
    end
  
    defp schedule_caching(), do: Process.send_after(self(), :cache, 60 * 1000) # In 1 hours
  
    def get_all(), do: GenServer.call(ImageEx.Bucket, {:get_all})
    def get(name), do: GenServer.call(ImageEx.Bucket, {:get, name})
    def put(key, value), do: GenServer.call(ImageEx.Bucket, {:put, key, value})
  
    # Handlers
    def handle_call({:get_all}, _from, state), do: {:reply, state["cached"], state}
    def handle_call({:get, name}, _from, state), do: {:reply, Map.get(state["cached"], name), state}
  
    def handle_info(:cache, state) do
      state = do_caching(state)
      schedule_caching()
      {:noreply, state}
    end
  end