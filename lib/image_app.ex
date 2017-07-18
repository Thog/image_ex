defmodule ImageEx.App do
  use Application
  import Supervisor.Spec, warn: false
  def start(_type, _args) do
    Supervisor.start_link([Plug.Adapters.Cowboy.child_spec(:http, ImageEx.HTTP,[], port: 4242),
                            supervisor(ImageEx.Bucket, [])], strategy: :one_for_one)
  end
end

defmodule ImageEx.API.Exceptions do
  defmacro __using__(_opts) do
    quote do @before_compile ImageEx.API.Exceptions end
  end
  defmacro __before_compile__(_) do
    quote location: :keep do
      defoverridable [call: 2]
      def call(conn, opts) do
        try do
          super(conn, opts)
        catch
          kind, reason ->
            stack = System.stacktrace
            reason = Exception.normalize(kind, reason, stack)
          status = case kind do x when x in [:error,:throw]-> Plug.Exception.status(reason); _-> 500 end
            conn |> Plug.Conn.put_resp_content_type("application/json")
            |> Plug.Conn.send_resp(status,Poison.encode!(%{state: "exception", reason: Exception.message(reason), trace: Exception.format(kind,reason,stack)}))
            :erlang.raise kind,reason,stack
        end
      end
    end
  end
end

defmodule ImageEx.HTTP do
  use Ewebmachine.Builder.Resources
  use Plug.Router
  use Plug.Builder
  require Logger
  plug Ewebmachine.Plug.Debug
  plug Plug.Logger
  plug :fetch_cookies
  plug :fetch_query_params
  plug :match
  plug :dispatch
  plug Plug.Parsers, parsers: [:urlencoded, :multipart]

  resources_plugs nomatch_404: true
  resource "/upload" do %{} after
    allowed_methods do: ["POST"]
    process_post do
      case conn.params do
        %{"upload" => upload} ->
          filename = :crypto.strong_rand_bytes(16) |> Base.encode16 |> String.downcase
          extention = String.split(upload.filename, ".") |> List.last
          case File.rename(upload.path, "#{Application.get_env(:image_ex, :path)}/bucket/#{filename}.#{extention}") do
            :ok -> {true,conn |> resp(200, "OK"),state}
            {:error, _} ->
              {true,conn |> resp(500, "FAIL"),state}
          end
        _ -> {true,conn |> resp(401, "FAIL"),state}
      end
    end
  end

  resource "/raw/*path" do %{path: Enum.join(path, "/")} after
    resource_exists do: File.regular?(path(state.path))
    content_types_provided do: [ {state.path |> Plug.MIME.path |> default_plain, :to_content} ]
    defh to_content, do: File.stream!( path(state.path), [], 300_000_000)
    defp path(relative), do: "#{Application.get_env(:image_ex, :path)}/bucket/#{relative}"
    defp default_plain(type), do: type
  end

  resource "/*path" do %{path: Enum.join(path, "/")} after
    content_types_provided do
      [user_agent] = Plug.Conn.get_req_header(conn, "user-agent")
      mime_type = state.path |> Plug.MIME.path
      case Enum.any?(["Twitterbot", "facebookexternalhit/", "Facebot"], &(String.contains?(user_agent, &1))) do
        true ->
          case Enum.any?(["image/", "video/", "audio/"], &(String.contains?(mime_type, &1))) do
            true -> {[{"text/html", :to_og}], conn, state |> put_in([:og_resource_type], mime_type |> String.split("/") |> Enum.at(0))}
            false -> {[{mime_type |> default_plain, :to_content}], conn,state}
          end
        false -> [{mime_type |> default_plain, :to_content}]
      end
    end

    resource_exists do
      case state[:og_resource_type] do
        nil -> {File.regular?(path(state.path)), conn, state}
        _ ->
          case ImageEx.Bucket.get(state.path) do
            nil -> {File.regular?(path(state.path)), conn, state |> Map.put(:redirection, "#{ImageEx.Utils.get_base_uri(conn)}raw/#{state.path}")}
            redirection -> {true, conn, state |> Map.put(:redirection, redirection)}
        end
      end
    end

    previously_existed do
      case ImageEx.Bucket.get(state.path) do
        nil -> false
        redirection -> {true, conn, state |> Map.put(:redirection, redirection)}
      end
    end

    moved_permanently do
      case state[:redirection] do
        nil -> false
        url -> {true, url}
      end
    end

    defh to_content, do: File.stream!( path(state.path), [], 300_000_000)
    defh to_og, do: generate_opengraph(conn, state, state.og_resource_type)
    defp path(relative), do: "#{Application.get_env(:image_ex, :path)}/bucket/#{relative}" |> IO.inspect
    defp default_plain(type), do: type
    
    def generate_opengraph(_, state, "image") do
      ImageEx.Utils.format_og(
        get_default_opengraph(state) ++ [
          {"twitter:card", "summary_large_image"},
          {"og:image", state.redirection},
        ])
    end
    
    def generate_opengraph(_, state, "video") do
      ImageEx.Utils.format_og(
        get_default_opengraph(state) ++ [
          {"og:video", state.redirection},
          {"og:video:type", state.redirection |> Plug.MIME.path},
        ])
    end

    def generate_opengraph(_, state, "audio") do
      ImageEx.Utils.format_og(
        get_default_opengraph(state) ++ [
          {"og:audio", state.redirection},
          {"og:audio:type", state.redirection |> Plug.MIME.path},
        ])
    end

    def get_default_opengraph(state) do
      [
        {"og:title", state.path},
        {"twitter:site", Application.get_env(:image_ex, :og_twitter)},
        {"twitter:creator", Application.get_env(:image_ex, :og_twitter)},
        {"og:description", Application.get_env(:image_ex, :og_description)}
      ]
    end
  end
end

defmodule ImageEx.Bucket do
  use GenServer
  require Logger

  @key Application.get_env(:image_ex, :puush_key)
  def start_link() do
    GenServer.start_link(__MODULE__, File.read!("#{Application.get_env(:image_ex, :path)}/app.json") |> Poison.decode!, [name: __MODULE__])
  end

  def init(state) do
    schedule_caching()
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
  def handle_call({:get_all}, _from, state), do: {:reply, state["cached"] |> IO.inspect, state}
  def handle_call({:get, name}, _from, state), do: {:reply, Map.get(state["cached"], name), state}

  def handle_info(:cache, state) do
    IO.inspect("Starting periodic caching...")
    state = do_caching(state)
    schedule_caching()
    IO.inspect("Periodic caching ended")
    {:noreply, state}
  end
end
