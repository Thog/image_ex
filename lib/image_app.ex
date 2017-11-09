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
  require Logger
  use Ewebmachine.Builder.Resources
  use Plug.Router
  use Plug.Builder
  plug Ewebmachine.Plug.Debug
  use ImageEx.API.Exceptions
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
      case conn.params |> IO.inspect do
        %{"upload" => upload} ->
          filename = :crypto.strong_rand_bytes(16) |> Base.encode16 |> String.downcase
          extention = String.split(upload.filename, ".") |> List.last
          case ImageEx.Utils.Crypto.decrypt_file_stream(upload.path) do
            {:ok,stream} ->
              stream |> Stream.run
              case File.rename(upload.path, "#{Application.get_env(:image_ex, :path)}/bucket/#{filename}.#{extention}") do
                :ok ->
                  {true,conn |> Plug.Conn.put_private(:resp_redirect, true) |> Plug.Conn.put_resp_header("location", "#{ImageEx.Utils.get_base_uri(conn)}/#{filename}.#{extention}"),state}
                {:error, _} ->
                  {true,conn |> resp(500, "FAIL"),state}
              end
            _ ->
              {true,conn |> resp(401, "FAIL"),state}
          end
        _ -> {true,conn |> resp(401, "FAIL"),state}
      end
    end
  end

  resource "/raw/*path" do %{path: Enum.join(path, "/")} after
    resource_exists do: File.regular?(path(state.path))
    content_types_provided do: [ {state.path |> MIME.from_path |> default_plain, :to_content} ]
    defh to_content do
      {:ok, stream} = ImageEx.Utils.Crypto.decrypt_file_stream(path(state.path))
      stream
    end
    defp path(relative), do: "#{Application.get_env(:image_ex, :path)}/bucket/#{relative}"
    defp default_plain(type), do: type
  end

  resource "/*path" do %{path: Enum.join(path, "/")} after
    content_types_provided do
      [user_agent] = Plug.Conn.get_req_header(conn, "user-agent")
      mime_type = state.path |> MIME.from_path
      case Enum.any?(["bot", "facebookexternalhit/"], &(String.contains?(user_agent, &1))) do
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

    defh to_content do
      {:ok, stream} = ImageEx.Utils.Crypto.decrypt_file_stream(path(state.path))
      stream
    end
    
    defh to_og, do: ImageEx.Utils.OpenGraph.generate_opengraph(conn, state, state.og_resource_type)
    defp path(relative), do: "#{Application.get_env(:image_ex, :path)}/bucket/#{relative}" |> IO.inspect
    defp default_plain(type), do: type
  end
end
